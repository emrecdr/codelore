
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
  // for users who opted out (WCAG 2.3.3 / prefers-reduced-motion).
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

  // Composite code-health band → resolved DaisyUI token for the
  // circle-pack canvas fill. green / yellow / red map to
  // success / warning / error so the bands theme-adapt in both light
  // and dark modes. This is the SAME `band` field (from the
  // code-health composite) the bivariate lens and its legend key off,
  // so the pure-health lens stays consistent with them. A path absent
  // from `data.code_health` (non-Tier-1 source, or the analysis was
  // skipped) gets the neutral grey the other lenses use for "no data".
  function bandLeafColor(band) {
    if (band === 'green')  return token('--color-success');
    if (band === 'yellow') return token('--color-warning');
    if (band === 'red')    return token('--color-error');
    return 'rgba(140, 140, 140, 0.55)';
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
  // without duplicating the feature-detection logic.
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

  // Map a band name to its DaisyUI theme token — never hardcoded hex.
  // Shared by the share bars, effort dot strip, and knowledge surfaces.
  function bandColor(band) {
    if (band === 'red')    return 'var(--color-error,   oklch(0.637 0.237 25.331))';
    if (band === 'yellow') return 'var(--color-warning, oklch(0.845 0.143 84.429))';
    return                        'var(--color-success, oklch(0.753 0.152 163.216))';
  }


