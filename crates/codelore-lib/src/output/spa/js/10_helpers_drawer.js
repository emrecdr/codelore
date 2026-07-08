
  // ═════════════════════════════════════════════════════════════════
  //  §4-§15 — FUNCTION DECLARATIONS (hoisted to IIFE scope)
  // ═════════════════════════════════════════════════════════════════


  // ─── §3d  Guided tour renderer ──────────────────────────────────
  //
  // Renders the stepper UI into #widget-guided-tour-body. Called at
  // boot (inactive state → Start button only) and after every step
  // transition (applyTourStep / exitTour in 00_setup_boot.js).
  // Pure DOM mutation — no ECharts, no CSS variable reads — so
  // rerender: false in the WIDGETS registry.
  //
  // Step transitions use CSS `transition: opacity` on the note banner;
  // the global `prefers-reduced-motion` rule in template.html clamps
  // all transition-durations to 0.01ms, so no animated movement occurs
  // for users who opted out (WCAG 2.3.3 / F138 pattern).
  function renderGuidedTour() {
    var mount = document.getElementById('widget-guided-tour-body');
    if (!mount) return;

    var isActive = (tourStep >= 0 && tourStep < TOUR_STEPS.length);
    var step = isActive ? TOUR_STEPS[tourStep] : null;

    // ── chip strip ────────────────────────────────────────────────
    var chipsHtml = '';
    for (var i = 0; i < TOUR_STEPS.length; i++) {
      var isCurrent = isActive && i === tourStep;
      var isDone    = isActive && i < tourStep;
      var chipClass = 'tour-chip' +
        (isCurrent ? ' tour-chip-active' : '') +
        (isDone    ? ' tour-chip-done'   : '');
      chipsHtml +=
        '<button type="button" class="' + chipClass + '"' +
          ' aria-label="Go to step ' + (i + 1) + ': ' + TOUR_STEPS[i].title + '"' +
          ' aria-current="' + (isCurrent ? 'step' : 'false') + '"' +
          ' data-tour-step="' + i + '">' +
          (i + 1) +
        '</button>';
    }

    // ── note banner (shown only during active tour) ───────────────
    var noteHtml = '';
    if (isActive && step) {
      noteHtml =
        '<div class="tour-note" role="status" aria-live="polite">' +
          '<span class="tour-note-title">' + escapeHtml(step.title) + '</span>' +
          ' — ' + escapeHtml(step.note) +
        '</div>';
    }

    // ── nav buttons ───────────────────────────────────────────────
    var prevDisabled = !isActive || tourStep === 0;
    var nextLabel    = (!isActive || tourStep === TOUR_STEPS.length - 1) ? 'Exit tour' : 'Next';
    var navHtml =
      '<div class="tour-nav">' +
        '<div class="tour-chips" role="list" aria-label="Tour steps">' +
          chipsHtml +
        '</div>' +
        '<div class="tour-buttons">' +
          (isActive
            ? '<button type="button" class="tour-btn" id="tour-prev"' +
                (prevDisabled ? ' disabled' : '') +
                ' aria-label="Previous tour step">Prev</button>'
            : '') +
          '<button type="button" class="tour-btn tour-btn-primary" id="tour-next">' +
            escapeHtml(isActive ? nextLabel : 'Start tour') +
          '</button>' +
          (isActive
            ? '<button type="button" class="tour-btn tour-btn-ghost" id="tour-exit">' +
                'Exit' +
              '</button>'
            : '') +
        '</div>' +
      '</div>' +
      noteHtml;

    mount.innerHTML = navHtml;

    // ── wire button handlers ──────────────────────────────────────
    var prevBtn = document.getElementById('tour-prev');
    var nextBtn = document.getElementById('tour-next');
    var exitBtn = document.getElementById('tour-exit');

    if (prevBtn) {
      prevBtn.addEventListener('click', function () {
        if (tourStep > 0) {
          tourStep -= 1;
          applyTourStep(tourStep);
        }
      });
    }
    if (nextBtn) {
      nextBtn.addEventListener('click', function () {
        if (!isActive) {
          // Start tour at step 0.
          tourStep = 0;
          applyTourStep(tourStep);
        } else if (tourStep >= TOUR_STEPS.length - 1) {
          // Final step → exit.
          exitTour();
        } else {
          tourStep += 1;
          applyTourStep(tourStep);
        }
      });
    }
    if (exitBtn) {
      exitBtn.addEventListener('click', function () {
        exitTour();
      });
    }

    // ── chip click handlers ───────────────────────────────────────
    var chips = mount.querySelectorAll('[data-tour-step]');
    for (var ci = 0; ci < chips.length; ci++) {
      chips[ci].addEventListener('click', (function (idx) {
        return function () {
          tourStep = idx;
          applyTourStep(tourStep);
        };
      }(parseInt(chips[ci].getAttribute('data-tour-step'), 10))));
    }
  }


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


  // Map a health score (0–100) to its color band ("green" / "yellow" / "red").
  // Thresholds are read from the run's options snapshot so the JS never
  // hardcodes them separately from the Rust constants in `bands.rs`.
  // Fallback values (70 / 40) match HEALTH_GREEN_MIN / HEALTH_YELLOW_MIN and
  // are applied when rendering older data payloads that pre-date this field.
  function bandFor(score, opts) {
    const greenMin = (opts && opts.health_green_min != null) ? opts.health_green_min : 70;
    const yellowMin = (opts && opts.health_yellow_min != null) ? opts.health_yellow_min : 40;
    if (score >= greenMin) return 'green';
    if (score >= yellowMin) return 'yellow';
    return 'red';
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

  // Drawer tab bar: DaisyUI tabs wired as an ARIA tablist. Overview is
  // the default selection on every open. Optional tabs (Health, X-Ray)
  // are injected only when the data is available for the current path.
  function drawerTabBar(hasHealthSeries, hasXray) {
    return '<div class="tabs tabs-bordered" role="tablist" aria-label="File detail sections">' +
      '<button type="button" class="tab tab-active" role="tab" id="drawer-tab-overview" aria-controls="drawer-panel-overview" aria-selected="true" tabindex="0">Overview</button>' +
      '<button type="button" class="tab" role="tab" id="drawer-tab-coupling" aria-controls="drawer-panel-coupling" aria-selected="false" tabindex="-1">Coupling</button>' +
      '<button type="button" class="tab" role="tab" id="drawer-tab-people" aria-controls="drawer-panel-people" aria-selected="false" tabindex="-1">People</button>' +
      (hasHealthSeries ? '<button type="button" class="tab" role="tab" id="drawer-tab-health" aria-controls="drawer-panel-health" aria-selected="false" tabindex="-1">Health</button>' : '') +
      (hasXray ? '<button type="button" class="tab" role="tab" id="drawer-tab-xray" aria-controls="drawer-panel-xray" aria-selected="false" tabindex="-1">X-Ray</button>' : '') +
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
    var healthHtml = '';
    var xrayHtml = '';

    // Build per-file health sparkline from file_health_series.
    const fileSeries = (d.file_health_series || []).filter(function (r) { return r.path === path; });
    if (fileSeries.length) {
      // Sort oldest-first for sparkline rendering order.
      fileSeries.sort(function (a, b) { return a.date < b.date ? -1 : a.date > b.date ? 1 : 0; });
      healthHtml += '<h4>Health over time</h4>';
      healthHtml += '<div id="drawer-health-sparkline" style="height: 140px; margin-bottom: 12px;"></div>';
      healthHtml += '<table class="table table-xs"><thead><tr><th>Date</th><th>Score</th><th>Band</th></tr></thead><tbody>';
      for (var hi = fileSeries.length - 1; hi >= 0; hi--) {
        const fs = fileSeries[hi];
        healthHtml += '<tr><td>' + escapeHtml(fs.date) + '</td><td>' + fmtNumberFlex(fs.score, 1) +
          '</td><td>' + escapeHtml(fs.band) + '</td></tr>';
      }
      healthHtml += '</tbody></table>';
    }

    // Build the X-Ray tab content from function_xray data for this path.
    // `function_xray` is an array of {path, rows} objects where rows are
    // FunctionXrayRow values: {function, change_freq, loc, cyclomatic, last_changed}.
    // The tab only appears when the backend computed xray rows for this path
    // (top-10 hotspot, Tier-1 language); otherwise the tab is omitted and
    // the Overview "Functions" section (from d.xray cognitive data) remains.
    var xrayEntry = (d.function_xray || []).find(function (e) { return e.path === path; });
    if (xrayEntry && xrayEntry.rows && xrayEntry.rows.length) {
      var xrows = xrayEntry.rows;
      // Max change_freq for proportional inline bar widths.
      var maxFreq = 0;
      for (var xri = 0; xri < xrows.length; xri++) {
        if ((xrows[xri].change_freq || 0) > maxFreq) maxFreq = xrows[xri].change_freq;
      }
      xrayHtml += '<table class="table table-xs" style="width:100%">' +
        '<thead><tr>' +
          '<th>Function</th>' +
          '<th style="min-width:90px">Change freq</th>' +
          '<th class="num">LOC</th>' +
          '<th class="num">CC</th>' +
        '</tr></thead><tbody>';
      for (var xfi = 0; xfi < xrows.length; xfi++) {
        var xr = xrows[xfi];
        var freqPct = maxFreq ? Math.round(((xr.change_freq || 0) / maxFreq) * 100) : 0;
        var barColor = freqPct >= 80 ? token('--color-error')
                     : freqPct >= 40 ? token('--color-warning')
                     : token('--color-base-content');
        xrayHtml += '<tr>' +
          '<td><code>' + escapeHtml(xr.function || '(anonymous)') + '</code></td>' +
          '<td>' +
            '<div style="display:flex;align-items:center;gap:4px;">' +
              '<div style="flex:1;height:6px;background:var(--color-base-200,#e5e7eb);border-radius:3px;overflow:hidden;">' +
                '<div style="width:' + freqPct + '%;height:100%;background:' + barColor + ';"></div>' +
              '</div>' +
              '<span style="min-width:22px;text-align:right;font-size:0.75em;">' + fmtInt(xr.change_freq) + '</span>' +
            '</div>' +
          '</td>' +
          '<td class="num">' + (xr.loc != null ? fmtInt(xr.loc) : '—') + '</td>' +
          '<td class="num">' + (xr.cyclomatic != null ? fmtInt(xr.cyclomatic) : '—') + '</td>' +
          '</tr>';
      }
      xrayHtml += '</tbody></table>';
    }

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

    // Section: marginal-owner risk chip (ownership × health signal)
    const mor = (d.marginal_owner_risk || []).find(function (r) { return r.path === path; });
    if (mor) {
      var morLabel = mor.risk === 'high' ? '⚠ High owner risk' : '⚠ Elevated owner risk';
      // Reuses ki-knowledge-loss-badge styling intentionally — same visual weight as the knowledge-loss chip.
      overviewHtml += '<div class="ki-knowledge-loss-badge" title="' + mor.note + '">' +
        morLabel + ' — top active share ' + fmtNumberFlex(mor.top_active_share, 2) +
        '</div>';
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

    const hasHealthSeries = healthHtml.length > 0;
    const hasXray = xrayHtml.length > 0;
    body.innerHTML =
      drawerTabBar(hasHealthSeries, hasXray) +
      drawerPanel('drawer-panel-overview', 'drawer-tab-overview', overviewInner, '') +
      drawerPanel('drawer-panel-coupling', 'drawer-tab-coupling', couplingHtml,
        'No change-coupling partners recorded for this file.') +
      drawerPanel('drawer-panel-people', 'drawer-tab-people', peopleHtml,
        'No ownership or contributor data for this file.') +
      (hasHealthSeries ? drawerPanel('drawer-panel-health', 'drawer-tab-health', healthHtml,
        'No health history for this file.') : '') +
      (hasXray ? drawerPanel('drawer-panel-xray', 'drawer-tab-xray', xrayHtml,
        'No function-level X-Ray data for this file.') : '');
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

    // Render the health sparkline after body.innerHTML so the container
    // exists. Only fires when the Health tab was injected above.
    if (hasHealthSeries) {
      try {
        renderDrawerHealthSparkline(path, d);
      } catch (e) {
        console.error('codelore: drawer health sparkline render failed for', path, e);
      }
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

  // Health sparkline in the drawer Health tab. Renders a compact line chart
  // of the file's score across sampled historical revisions using ECharts.
  // The container `drawer-health-sparkline` is only present when
  // `file_health_series` contains entries for this path (i.e., it's a top
  // hotspot). Returns immediately if ECharts is unavailable.
  function renderDrawerHealthSparkline(path, d) {
    const container = document.getElementById('drawer-health-sparkline');
    if (!container || typeof window.echarts === 'undefined') return;
    const series = (d.file_health_series || [])
      .filter(function (r) { return r.path === path; })
      .sort(function (a, b) { return a.date < b.date ? -1 : a.date > b.date ? 1 : 0; });
    if (!series.length) return;
    const greenMin = (d.options && typeof d.options.health_green_min === 'number')
      ? d.options.health_green_min : 70;
    const yellowMin = (d.options && typeof d.options.health_yellow_min === 'number')
      ? d.options.health_yellow_min : 40;
    const chart = mountEcharts(container);
    chart.setOption({
      animation: false,
      grid: { top: 8, bottom: 24, left: 36, right: 8 },
      xAxis: { type: 'category', data: series.map(function (r) { return r.date; }), axisLabel: { fontSize: 10 } },
      yAxis: { type: 'value', min: 0, max: 100, splitLine: { show: true },
        axisLabel: { fontSize: 10 } },
      series: [{
        type: 'line',
        data: series.map(function (r) { return r.score; }),
        smooth: true,
        symbol: 'circle',
        symbolSize: 5,
        lineStyle: { width: 2, color: token('--cl-health-green') },
        itemStyle: { color: function (p) {
          const v = p.value;
          return v >= greenMin ? token('--cl-health-green')
            : v >= yellowMin ? token('--cl-health-yellow')
            : token('--cl-health-red');
        } },
        markLine: {
          silent: true,
          symbol: 'none',
          label: { show: false },
          lineStyle: { type: 'dashed', opacity: 0.4 },
          data: [
            { yAxis: greenMin, lineStyle: { color: token('--cl-health-green') } },
            { yAxis: yellowMin, lineStyle: { color: token('--cl-health-yellow') } },
          ],
        },
      }],
      tooltip: { trigger: 'axis', formatter: function (ps) {
        return ps[0].axisValue + ': ' + ps[0].value.toFixed(1);
      } },
    });
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
        const healthBand = bandFor(median, data.options || {});
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

  // ─── Improvements feed widget ─────────────────────────────────────
  // Renders `health_transitions` (signal-bearing band transitions) as
  // two lists: recent improvements ↑ and regressions ↓. Each row is
  // clickable via `_codeloreShowDetail` for linked brushing.
  function renderImprovementsFeed(transitions) {
    const container = document.getElementById('widget-improvements-feed-body');
    if (!container) return;
    if (!transitions || !transitions.length) {
      container.innerHTML = '<div class="empty">No band transitions detected across the sampled history.</div>';
      return;
    }
    // `transitions` is newest-first from the Rust emitter.
    const improved = transitions.filter(function (r) { return r.direction === 'improved'; }).slice(0, 8);
    const regressed = transitions.filter(function (r) { return r.direction === 'regressed'; }).slice(0, 8);

    function makeList(rows, label, icon) {
      if (!rows.length) return '';
      var html = '<h4 class="feed-section-title">' + icon + ' ' + escapeHtml(label) + '</h4><ul class="feed-list">';
      for (var i = 0; i < rows.length; i++) {
        const r = rows[i];
        const shortPath = r.path.split('/').pop() || r.path;
        html += '<li class="feed-row" data-path="' + escapeHtml(r.path) + '" tabindex="0" role="button">' +
          '<code title="' + escapeHtml(r.path) + '">' + escapeHtml(shortPath) + '</code>' +
          ' <span class="feed-band">' + escapeHtml(r.from_band) + ' → ' + escapeHtml(r.to_band) + '</span>' +
          ' <span class="feed-date">' + escapeHtml(r.date) + '</span>' +
          '</li>';
      }
      html += '</ul>';
      return html;
    }

    container.innerHTML =
      makeList(improved, 'Recent improvements', '↑') +
      makeList(regressed, 'Regressions', '↓');

    // Wire clicks + keyboard activation for linked brushing.
    const rows = container.querySelectorAll('.feed-row');
    for (var ri = 0; ri < rows.length; ri++) {
      rows[ri].addEventListener('click', function (evt) {
        const p = evt.currentTarget.getAttribute('data-path');
        if (window._codeloreShowDetail) window._codeloreShowDetail(p);
      });
      rows[ri].style.cursor = 'pointer';
      wireRowKbActivation(rows[ri]);
    }
  }

  // ─── Factor header widget ────────────────────────────────────────────
  // Renders the four-factor (Code, Architecture, Knowledge, Delivery)
  // overview header above the KPI tiles. Each tile shows:
  //   • A bullet bar — band-colored track (full width), a marker at the
  //     headline value, and a baseline tick at the series mean.
  //   • A 60px ECharts sparkline driven by `tile.series`.
  //   • An "Attention" chip only when `tile.attention == true`.
  //
  // Band color is read from CSS custom properties via `token()` so the
  // widget re-renders on theme switch (registered as `rerender: 'theme'`
  // in WIDGETS). Thresholds are read from `data.options` — never
  // hardcoded here.
  function renderFactorHeader(factors, opts) {
    const container = document.getElementById('widget-factor-header-body');
    if (!container) return;
    if (!factors || !factors.length) {
      container.innerHTML = '<div class="empty">No factor data — run with health-trend enabled.</div>';
      return;
    }

    const o = opts || {};
    const greenMin = typeof o.health_green_min === 'number' ? o.health_green_min : 70;
    const yellowMin = typeof o.health_yellow_min === 'number' ? o.health_yellow_min : 40;

    function bandColor(score) {
      if (score === null || score === undefined) return token('--cl-health-yellow');
      return score >= greenMin ? token('--cl-health-green')
           : score >= yellowMin ? token('--cl-health-yellow')
           : token('--cl-health-red');
    }

    function bulletBar(tile) {
      const val = tile.headline !== null && tile.headline !== undefined ? tile.headline : 0;
      const seriesMean = tile.series && tile.series.length
        ? tile.series.reduce(function (s, v) { return s + v; }, 0) / tile.series.length
        : val;
      const color = bandColor(tile.headline);
      // Track = full-width bar; marker = filled circle at headline %;
      // baseline tick = thin line at series mean %.
      return '<div class="factor-bullet-wrap" aria-label="' + fmtNumberFlex(val, 1) + ' / 100">' +
        '<div class="factor-bullet-track">' +
          '<div class="factor-bullet-fill" style="width:' + Math.min(100, Math.max(0, val)) + '%;background:' + color + ';"></div>' +
          '<div class="factor-bullet-mean-tick" style="left:' + Math.min(100, Math.max(0, seriesMean)) + '%;"></div>' +
        '</div>' +
        '<span class="factor-bullet-label" style="color:' + color + ';">' + fmtNumberFlex(val, 1) + '</span>' +
        '</div>';
    }

    // Delivery tile (headline=null) renders a key-value numbers list.
    // Band color on the first number (rework %) uses rework-specific thresholds
    // — NOT the generic health band — because the Pluralsight benchmark range
    // (green <9 %, yellow 9-14 %, red ≥15 %) differs from the health scale.
    var REWORK_BAND_COLORS = {
      green: 'var(--cl-health-green, oklch(70% 0.18 145))',
      yellow: 'var(--cl-health-yellow, oklch(80% 0.16 85))',
      red: 'var(--cl-health-red, oklch(60% 0.20 25))',
    };

    function numbersList(tile) {
      var bandColor = REWORK_BAND_COLORS[tile.band] || '';
      var html = '<div class="factor-numbers">';
      var nums = tile.numbers || [];
      for (var ni = 0; ni < nums.length; ni++) {
        var pair = nums[ni];
        var label = escapeHtml(pair[0] || '');
        var value = escapeHtml(pair[1] || '');
        // Only the first number (rework %) gets band coloring.
        var valStyle = (ni === 0 && bandColor) ? ' style="color:' + bandColor + ';"' : '';
        html += '<div class="factor-number-row">' +
          '<span class="factor-number-label">' + label + '</span>' +
          '<span class="factor-number-value"' + valStyle + '>' + value + '</span>' +
          '</div>';
      }
      html += '</div>';
      return html;
    }

    var html = '<div class="factor-tiles">';
    for (var i = 0; i < factors.length; i++) {
      const t = factors[i];
      const hasHeadline = t.headline !== null && t.headline !== undefined;
      const headlineStr = hasHeadline ? fmtNumberFlex(t.headline, 1) : null;
      const hasNumbers = t.numbers && t.numbers.length > 0;
      html += '<div class="factor-tile" id="factor-tile-' + i + '">' +
        '<div class="factor-name">' + escapeHtml(t.name) + '</div>' +
        (t.attention ? '<span class="factor-attention-chip">Attention</span>' : '') +
        (headlineStr !== null
          ? '<div class="factor-headline">' + headlineStr + '</div>' + bulletBar(t)
          : (hasNumbers ? numbersList(t) : '<div class="factor-headline">—</div>')) +
        (t.series && t.series.length ? '<div id="factor-sparkline-' + i + '" class="factor-sparkline"></div>' : '') +
        '<div class="factor-detail">' + escapeHtml(t.detail || '') + '</div>' +
        '</div>';
    }
    html += '</div>';
    container.innerHTML = html;

    // Render per-tile ECharts sparklines after DOM is set.
    for (var j = 0; j < factors.length; j++) {
      (function (idx, tile) {
        // Delivery tile and other no-series tiles have no sparkline element.
        var el = document.getElementById('factor-sparkline-' + idx);
        if (!el || !tile.series || !tile.series.length || typeof window.echarts === 'undefined') return;
        try {
          var chart = mountEcharts(el);
          chart.setOption({
            animation: false,
            grid: { top: 2, bottom: 2, left: 2, right: 2 },
            xAxis: { type: 'category', show: false, data: tile.series.map(function (_, k) { return k; }) },
            yAxis: { type: 'value', show: false, min: 0, max: 100 },
            series: [{
              type: 'line',
              data: tile.series,
              smooth: true,
              symbol: 'none',
              lineStyle: { width: 1.5, color: bandColor(tile.headline) },
            }],
          });
        } catch (e) {
          console.error('codelore: factor sparkline render failed for', tile.name, e);
        }
      })(j, factors[j]);
    }
  }

  // ─── §A.5  Widget: banded share bars + effort dot strip ─────────────
  //
  // Renders two 100% stacked horizontal bars (LOC share and churn share
  // per code-health band) plus a 20-dot effort strip where each dot
  // represents 5% of trailing-window churn. All HTML/CSS — no ECharts.
  //
  // Band colours come from CSS custom properties set by the active
  // DaisyUI theme (error/warning/success), resolved at render time so the
  // widget re-colours correctly on theme toggle (rerender: 'theme').
  // Accessibility: role="img" + aria-label on every bar and the dot strip;
  // percentage text labels inside segments serve as non-colour redundant
  // cues (WCAG 1.4.1 — Use of Color).
  //
  // Caption: "X% of the last {window} days' changes landed in red code"
  // with Wilson 95% CI expressed as a title/tooltip attribute.
  function renderShareBars(rows, opts) {
    var mount = document.getElementById('widget-share-bars-body');
    if (!mount) return;
    if (!rows || !rows.length) {
      mount.innerHTML =
        '<p class="text-base-content/50 text-sm">No effort-exposure data available.</p>';
      return;
    }

    var BAND_ORDER = ['red', 'yellow', 'green'];
    var BAND_LABEL = { red: 'Red', yellow: 'Yellow', green: 'Green' };

    // Band colours from DaisyUI theme tokens — never hardcoded hex.
    function bandColor(band) {
      if (band === 'red')    return 'var(--color-error,   oklch(0.637 0.237 25.331))';
      if (band === 'yellow') return 'var(--color-warning, oklch(0.845 0.143 84.429))';
      return                        'var(--color-success, oklch(0.753 0.152 163.216))';
    }

    // Index rows by band for O(1) lookup.
    var byBand = {};
    for (var i = 0; i < rows.length; i++) {
      byBand[rows[i].band] = rows[i];
    }

    // ── Bar builder ────────────────────────────────────────────────────
    // Builds one 100%-stacked horizontal bar using valueFn(row) → %.
    // Segments narrower than 0.5% are hidden (invisible sliver).
    // Text labels appear inside segments ≥8% wide (non-colour cue).
    function buildBar(axisLabel, valueFn, ariaDesc) {
      var total = 0;
      for (var b = 0; b < BAND_ORDER.length; b++) {
        var r = byBand[BAND_ORDER[b]];
        total += r ? (valueFn(r) || 0) : 0;
      }
      var segments = '';
      for (var bi = 0; bi < BAND_ORDER.length; bi++) {
        var band = BAND_ORDER[bi];
        var row = byBand[band];
        var raw = row ? (valueFn(row) || 0) : 0;
        var pct = total > 0 ? (raw / total * 100) : 0;
        if (pct < 0.5) continue;
        var pctStr = pct.toFixed(1);
        segments +=
          '<div class="share-bar-segment"' +
              ' style="width:' + pct.toFixed(2) + '%;background:' + bandColor(band) + ';"' +
              ' title="' + BAND_LABEL[band] + ': ' + pctStr + '%">' +
            (pct >= 8
              ? '<span class="share-bar-label">' + BAND_LABEL[band] + ' ' + pctStr + '%</span>'
              : '') +
          '</div>';
      }
      return (
        '<div class="share-bar-row">' +
          '<span class="share-bar-axis-label">' + axisLabel + '</span>' +
          '<div class="share-bar-track" role="img" aria-label="' + ariaDesc + '">' +
            segments +
          '</div>' +
        '</div>'
      );
    }

    var locBar = buildBar(
      'LOC',
      function (r) { return r.loc_share_pct; },
      'Source lines of code share per health band: ' +
        BAND_ORDER.map(function (b) {
          var r = byBand[b]; return BAND_LABEL[b] + ' ' + (r ? r.loc_share_pct.toFixed(1) : '0') + '%';
        }).join(', ')
    );
    var churnBar = buildBar(
      'Churn',
      function (r) { return r.churn_share_pct; },
      'Churn share per health band in the trailing window: ' +
        BAND_ORDER.map(function (b) {
          var r = byBand[b]; return BAND_LABEL[b] + ' ' + (r ? r.churn_share_pct.toFixed(1) : '0') + '%';
        }).join(', ')
    );

    // ── Caption beneath the churn bar ─────────────────────────────────
    var captionHtml = '';
    var redRow = byBand['red'];
    if (redRow) {
      var redChurn = (redRow.churn_share_pct || 0).toFixed(1);
      var ciLo = ((redRow.commit_share_ci_low  || 0) * 100).toFixed(1);
      var ciHi = ((redRow.commit_share_ci_high || 0) * 100).toFixed(1);
      var windowStr = (opts && opts.window_days)
        ? 'the last ' + opts.window_days + ' days’'
        : 'recent';
      captionHtml =
        '<p class="share-bars-caption"' +
            ' title="Wilson 95 % CI on commit share: [' + ciLo + ' %, ' + ciHi + ' %]">' +
          '<strong>' + redChurn + '%</strong> of ' + windowStr + ' changes landed in red code' +
        '</p>';
    }

    // ── 20-dot effort strip ────────────────────────────────────────────
    // Each dot represents 5% of total window churn, coloured by band.
    // Dot counts: round(churn_share_pct / 5) per band; remainder (to
    // reach exactly 20) goes to the band with the largest raw share.
    var dotCounts = {};
    var dotSum = 0;
    var maxBand = null;
    var maxShare = -1;
    for (var di = 0; di < BAND_ORDER.length; di++) {
      var db = BAND_ORDER[di];
      var dr = byBand[db];
      var share = dr ? (dr.churn_share_pct || 0) : 0;
      var rounded = Math.round(share / 5);
      dotCounts[db] = rounded;
      dotSum += rounded;
      if (share > maxShare) { maxShare = share; maxBand = db; }
    }
    var remainder = 20 - dotSum;
    if (remainder !== 0 && maxBand !== null) {
      dotCounts[maxBand] = (dotCounts[maxBand] || 0) + remainder;
    }

    var dots = '';
    for (var dbi = 0; dbi < BAND_ORDER.length; dbi++) {
      var dotBand = BAND_ORDER[dbi];
      var count = Math.max(0, dotCounts[dotBand] || 0);
      for (var k = 0; k < count; k++) {
        dots +=
          '<span class="effort-dot"' +
              ' style="background:' + bandColor(dotBand) + ';"' +
              ' title="' + BAND_LABEL[dotBand] + ' band (5% churn per dot)">' +
          '</span>';
      }
    }
    var dotAriaLabel =
      (dotCounts['red'] || 0) + ' red, ' +
      (dotCounts['yellow'] || 0) + ' yellow, ' +
      (dotCounts['green'] || 0) + ' green — each dot = 5% of window churn';
    var dotStrip =
      '<div class="effort-dot-strip-wrap">' +
        '<span class="share-bar-axis-label">Effort</span>' +
        '<div class="effort-dot-strip" role="img" aria-label="' + dotAriaLabel + '">' +
          dots +
        '</div>' +
      '</div>';

    mount.innerHTML =
      '<div class="share-bars-container">' +
        locBar +
        churnBar +
        dotStrip +
        captionHtml +
      '</div>';
  }

  // ─── §18 Knowledge surfaces widget ────────────────────────────────────
  //
  // Renders three panels into #widget-knowledge-surfaces-body:
  //   1. Familiarity bullet bars — team familiarity % and islands % using
  //      bandFor() colour coding (same thresholds as code-health bands).
  //   2. Team-composition stacked bar — onboarded / experienced / veteran
  //      commit share as a proportional bar.
  //   3. Coordination table — top-10 files by tier desc then entropy desc,
  //      clickable to open the file-detail drawer.
  function renderKnowledgeSurfaces(famRows, teamRows, coordRows) {
    var mount = document.getElementById('widget-knowledge-surfaces-body');
    if (!mount) return;

    var html = '';

    // ── 1. Familiarity bullet bars ─────────────────────────────────────
    var fam = famRows && famRows.length ? famRows[0] : null;
    if (fam) {
      var famPct = typeof fam.familiarity_pct === 'number' ? fam.familiarity_pct : 0;
      var islPct = typeof fam.islands_pct === 'number' ? fam.islands_pct : 0;
      var activeAuthors = typeof fam.active_authors === 'number' ? fam.active_authors : '—';
      // Familiarity: higher is better → green ≥ 70, yellow ≥ 40, red < 40
      var famColor = bandColor(famPct >= 70 ? 'green' : famPct >= 40 ? 'yellow' : 'red');
      // Islands: lower is better → green ≤ 20 %, yellow ≤ 40 %, red > 40 %
      var islColor = bandColor(islPct <= 20 ? 'green' : islPct <= 40 ? 'yellow' : 'red');
      html +=
        '<div class="knowledge-familiarity-bars">' +
          '<div class="share-bar-row" title="Mean team familiarity with active files (' + fmtNumberFlex(famPct, 1) + '%)">' +
            '<span class="share-bar-axis-label">Familiarity</span>' +
            '<div class="share-bar-track">' +
              '<div class="share-bar-fill" style="width:' + Math.min(100, famPct) + '%;background:' + famColor + ';"></div>' +
            '</div>' +
            '<span class="share-bar-value">' + fmtNumberFlex(famPct, 1) + '%</span>' +
          '</div>' +
          '<div class="share-bar-row" title="Knowledge islands: files with no active owner (' + fmtNumberFlex(islPct, 1) + '% of files)">' +
            '<span class="share-bar-axis-label">Islands</span>' +
            '<div class="share-bar-track">' +
              '<div class="share-bar-fill" style="width:' + Math.min(100, islPct) + '%;background:' + islColor + ';"></div>' +
            '</div>' +
            '<span class="share-bar-value">' + fmtNumberFlex(islPct, 1) + '%</span>' +
          '</div>' +
          '<p class="knowledge-caption">Active authors: <strong>' + activeAuthors + '</strong>' +
            (fam.verdict ? ' — <em>' + escapeHtml(fam.verdict) + '</em>' : '') +
          '</p>' +
        '</div>';
    }

    // ── 2. Team-composition stacked bar ───────────────────────────────
    if (teamRows && teamRows.length) {
      // Collect commit share per bucket; fall back to 0 if bucket absent.
      var bucketColors = {
        onboarded:  'var(--color-info,    oklch(0.623 0.214 259.532))',
        experienced:'var(--color-success, oklch(0.753 0.152 163.216))',
        veteran:    'var(--color-primary, oklch(0.491 0.270 282.717))',
      };
      var totalShare = 0;
      var segments = '';
      var legend = '';
      for (var ti = 0; ti < teamRows.length; ti++) {
        var tr = teamRows[ti];
        var share = typeof tr.commit_share_pct === 'number' ? tr.commit_share_pct : 0;
        totalShare += share;
        var color = bucketColors[tr.bucket] || getCssVar('--p');
        segments +=
          '<div class="team-bar-segment" style="width:' + fmtNumberFlex(share, 1) + '%;background:' + color + ';"' +
            ' title="' + escapeHtml(tr.bucket) + ': ' + fmtNumberFlex(share, 1) + '% of commits, ' + tr.active_authors + ' author(s)">' +
          '</div>';
        legend +=
          '<span class="team-bar-key" style="color:' + color + ';">' + escapeHtml(tr.bucket) + '</span> ' +
          fmtNumberFlex(share, 1) + '% (' + tr.active_authors + ')  ';
      }
      html +=
        '<div class="team-composition-bar">' +
          '<div class="team-bar-track" role="img" aria-label="Team tenure distribution">' + segments + '</div>' +
          '<p class="knowledge-caption">' + legend.trim() + '</p>' +
        '</div>';
    }

    // ── 3. Coordination table ──────────────────────────────────────────
    if (coordRows && coordRows.length) {
      var tierBadge = function (t) {
        var colors = { high: 'badge-error', medium: 'badge-warning', low: 'badge-info', single: 'badge-ghost' };
        return '<span class="badge badge-sm ' + (colors[t] || 'badge-ghost') + '">' + escapeHtml(t) + '</span>';
      };
      var rows = '';
      for (var ci = 0; ci < coordRows.length; ci++) {
        var cr = coordRows[ci];
        var name = (cr.path || '').split('/').pop();
        rows +=
          '<tr class="hover" style="cursor:pointer;" onclick="window._codeloreShowDetail && window._codeloreShowDetail(' + JSON.stringify(cr.path) + ')">' +
            '<td class="coord-path" title="' + escapeHtml(cr.path || '') + '">' + escapeHtml(name) + '</td>' +
            '<td>' + tierBadge(cr.tier || 'single') + '</td>' +
            '<td>' + fmtNumberFlex(cr.fragmentation, 2) + '</td>' +
            '<td>' + fmtNumberFlex(cr.cochange_entropy, 3) + '</td>' +
          '</tr>';
      }
      html +=
        '<div class="table-container coordination-table">' +
          '<table class="table table-xs">' +
            '<thead><tr>' +
              '<th>File</th><th>Tier</th><th>Fragmentation</th><th>Entropy</th>' +
            '</tr></thead>' +
            '<tbody>' + rows + '</tbody>' +
          '</table>' +
        '</div>';
    }

    if (!html) {
      html = '<p class="muted-hint">No knowledge data — run with source files present to populate.</p>';
    }

    mount.innerHTML = html;
  }

