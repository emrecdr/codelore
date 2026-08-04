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
    const healthHtml = drawerHealthSeriesHtml(path, d);
    const xrayHtml = drawerXrayHtml(path, d);

    // All section lookups are wrapped so one row's malformed data can't
    // blank the drawer: partial html built before any throw is still shown,
    // and the underlying error is surfaced to the console for diagnosis. Each
    // section is a pure `(path, d) → html` builder; the target accumulator at
    // the call site keeps its Overview / Coupling / People routing, and the
    // original build order is preserved.
    try {
      overviewHtml += drawerHotspotHtml(path, d);
      peopleHtml += drawerKnowledgeIslandHtml(path, d);
      // Partner + contributor rows share the departed-author signal and the
      // window-global primary-author map; compute both once, at their original
      // position, and pass them down explicitly.
      const primaryAuthorByPath = window._codelorePrimaryAuthorByPath || {};
      const departedSet = (window.Alpine && window.Alpine.store && window.Alpine.store('scenario'))
        ? new Set(window.Alpine.store('scenario').departed)
        : new Set();
      couplingHtml += drawerCouplingHtml(path, d, primaryAuthorByPath, departedSet);
      peopleHtml += drawerContributorsHtml(path, d, departedSet);
      overviewHtml += drawerFunctionsHtml(path, d);
      overviewHtml += drawerClonesHtml(path, d);
      overviewHtml += drawerCodeHealthHtml(path, d);
      overviewHtml += drawerMarginalOwnerRiskHtml(path, d);
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

  // ─── Drawer section builders ──────────────────────────────────────
  // Each is a pure `(path, d) → html` function extracted from
  // showFileDetailDrawer so no single unit carries the whole drawer's
  // complexity. They hold no state and append to nothing: the parent owns
  // the accumulators and the Overview / Coupling / People routing. An empty
  // section returns '' (the parent's panel builder then renders the muted
  // empty-state message).

  // Health-over-time table + sparkline mount. Built outside the section
  // try/catch, exactly as before, so its failure semantics are unchanged.
  // A non-empty return is what drives the Health tab's presence.
  function drawerHealthSeriesHtml(path, d) {
    const fileSeries = (d.file_health_series || []).filter(function (r) { return r.path === path; });
    if (!fileSeries.length) return '';
    // Sort oldest-first for sparkline rendering order.
    fileSeries.sort(function (a, b) { return a.date < b.date ? -1 : a.date > b.date ? 1 : 0; });
    var html = '<h4>Health over time</h4>';
    html += '<div id="drawer-health-sparkline" style="height: 140px; margin-bottom: 12px;"></div>';
    html += '<table class="table table-xs"><thead><tr><th scope="col">Date</th><th scope="col">Score</th><th scope="col">Band</th></tr></thead><tbody>';
    for (var hi = fileSeries.length - 1; hi >= 0; hi--) {
      const fs = fileSeries[hi];
      html += '<tr><td>' + escapeHtml(fs.date) + '</td><td>' + fmtNumberFlex(fs.score, 1) +
        '</td><td>' + escapeHtml(fs.band) + '</td></tr>';
    }
    html += '</tbody></table>';
    return html;
  }

  // X-Ray tab content from function_xray for this path. `function_xray` is
  // an array of {path, rows} objects where rows are FunctionXrayRow values:
  // {function, change_freq, loc, cyclomatic, last_changed}. Returns '' unless
  // the backend computed xray rows for this path (top-10 hotspot, Tier-1
  // language); the Overview "Functions" section (from d.xray cognitive data)
  // renders separately regardless.
  function drawerXrayHtml(path, d) {
    var xrayEntry = (d.function_xray || []).find(function (e) { return e.path === path; });
    if (!xrayEntry || !xrayEntry.rows || !xrayEntry.rows.length) return '';
    var xrows = xrayEntry.rows;
    // Max change_freq for proportional inline bar widths.
    var maxFreq = 0;
    for (var xri = 0; xri < xrows.length; xri++) {
      if ((xrows[xri].change_freq || 0) > maxFreq) maxFreq = xrows[xri].change_freq;
    }
    var html = '<table class="table table-xs" style="width:100%">' +
      '<thead><tr>' +
        '<th scope="col">Function</th>' +
        '<th scope="col" style="min-width:90px">Change freq</th>' +
        '<th scope="col" class="num">LOC</th>' +
        '<th scope="col" class="num">CC</th>' +
      '</tr></thead><tbody>';
    for (var xfi = 0; xfi < xrows.length; xfi++) {
      var xr = xrows[xfi];
      var freqPct = maxFreq ? Math.round(((xr.change_freq || 0) / maxFreq) * 100) : 0;
      var barColor = freqPct >= 80 ? token('--color-error')
                   : freqPct >= 40 ? token('--color-warning')
                   : token('--color-base-content');
      html += '<tr>' +
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
    html += '</tbody></table>';
    return html;
  }

  // Overview: hotspot row.
  function drawerHotspotHtml(path, d) {
    const hot = (d.hotspots || []).find(function (r) { return r.path === path; });
    if (!hot) return '';
    return '<h4>Hotspot</h4><dl>' +
      '<dt>Revisions</dt><dd>' + fmtInt(hot.revisions) + '</dd>' +
      '<dt>Cognitive</dt><dd>' + fmtNumberFlex(hot.cognitive, 0) + '</dd>' +
      '<dt>Cognitive health</dt><dd>' + fmtNumberFlex(hot.cognitive_health, 1) + '</dd>' +
      '<dt>Hotspot score</dt><dd>' + fmtNumberFlex(hot.hotspot_score, 2) + '</dd>' +
      '</dl>';
  }

  // People: knowledge island. Payload uses `entity` here (not `path` like
  // the other tables) — check both so the lookup succeeds regardless of
  // which field carries the path.
  function drawerKnowledgeIslandHtml(path, d) {
    const ki = (d.knowledge_islands || []).find(function (r) {
      return (r.path || r.entity) === path;
    });
    if (!ki) return '';
    return '<h4>Knowledge island</h4><dl>' +
      '<dt>Primary author</dt><dd>' + escapeHtml(ki.main_author || '') + '</dd>' +
      '<dt>Ownership</dt><dd>' + fmtNumberFlex(ki.ownership_pct, 1) + ' %</dd>' +
      '<dt>Days since active</dt><dd>' + fmtInt(ki.days_since_main_active) + '</dd>' +
      '<dt>Total LoC</dt><dd>' + fmtInt(ki.total_loc) + '</dd>' +
      '</dl>';
  }

  // Coupling: change-coupling partners. Each partner is annotated with its
  // primary author and (when scenario.departed contains that author) a
  // knowledge-loss badge — the same reactive signal the hotspot table + KI
  // list use. The primary-author map and departed set are computed once by
  // the caller and passed in.
  function drawerCouplingHtml(path, d, primaryAuthorByPath, departedSet) {
    const partners = (d.coupling || []).filter(function (r) {
      return r.entity_a === path || r.entity_b === path;
    });
    if (!partners.length) return '';
    var html = '<h4>Coupling partners</h4><ul class="drawer-partners">';
    for (var i = 0; i < Math.min(partners.length, 20); i++) {
      const p = partners[i];
      const other = (p.entity_a === path) ? p.entity_b : p.entity_a;
      const partnerAuthor = primaryAuthorByPath[other] || '';
      const isDeparted = partnerAuthor && departedSet.has(partnerAuthor);
      html += '<li' + (isDeparted ? ' class="drawer-partner-departed"' : '') + '>' +
        '<code>' + escapeHtml(other) + '</code>' +
        ' — ' + fmtInt(p.shared) + ' shared revs' +
        (p.degree != null ? (' (' + fmtNumberFlex(p.degree, 1) + '% coupling)') : '') +
        (partnerAuthor ? ' <span class="drawer-author">' + escapeHtml(partnerAuthor) + '</span>' : '') +
        (isDeparted ? ' <span class="ki-knowledge-loss-badge">knowledge-loss</span>' : '') +
        '</li>';
    }
    if (partners.length > 20) {
      html += '<li>… ' + (partners.length - 20) + ' more</li>';
    }
    html += '</ul>';
    return html;
  }

  // People: top contributors. Aggregates entity_ownership rows for the file
  // by author and ranks by total LoC contribution. Answers "who else has
  // touched this besides the primary author?" — the bus-factor recovery
  // question — without leaving the drawer.
  function drawerContributorsHtml(path, d, departedSet) {
    const contribRows = (d.entity_ownership || []).filter(function (r) {
      return r.entity === path;
    });
    if (!contribRows.length) return '';
    const byAuthor = {};
    for (var ci = 0; ci < contribRows.length; ci++) {
      const r = contribRows[ci];
      if (!byAuthor[r.author]) byAuthor[r.author] = { added: 0, deleted: 0 };
      byAuthor[r.author].added += (r.added || 0);
      byAuthor[r.author].deleted += (r.deleted || 0);
    }
    // Drop zero-contribution entries. Entity_ownership keeps a row for any
    // commit that touched the path; renames / reverts can net to 0 added +
    // 0 deleted and produce misleading "0%" contributors (especially when
    // flagged as knowledge-loss — if they didn't contribute lines, their
    // departure doesn't actually lose knowledge of this file). Only show
    // authors with substantive contribution.
    const contribList = Object.keys(byAuthor)
      .filter(function (a) { return (byAuthor[a].added + byAuthor[a].deleted) > 0; })
      .map(function (a) {
        return { author: a, added: byAuthor[a].added, deleted: byAuthor[a].deleted };
      })
      .sort(function (a, b) {
        return (b.added + b.deleted) - (a.added + a.deleted);
      });
    if (!contribList.length) return '';
    const total = contribList.reduce(function (acc, r) { return acc + r.added + r.deleted; }, 0) || 1;
    var html = '<h4>Top contributors</h4><ul class="drawer-partners">';
    for (var pi = 0; pi < Math.min(contribList.length, 5); pi++) {
      const c = contribList[pi];
      const pct = Math.round(((c.added + c.deleted) / total) * 100);
      const cDeparted = departedSet.has(c.author);
      html += '<li' + (cDeparted ? ' class="drawer-partner-departed"' : '') + '>' +
        escapeHtml(c.author) +
        ' — ' + pct + '% (<span class="drawer-author">+' + fmtInt(c.added) + ' / -' + fmtInt(c.deleted) + '</span>)' +
        (cDeparted ? ' <span class="ki-knowledge-loss-badge">knowledge-loss</span>' : '') +
        '</li>';
    }
    if (contribList.length > 5) {
      html += '<li>… ' + (contribList.length - 5) + ' more contributors</li>';
    }
    html += '</ul>';
    return html;
  }

  // Overview: functions (from X-ray complexity scan). Lists the file's
  // top-complexity functions inline so the user doesn't have to drill into
  // the sunburst widget separately.
  function drawerFunctionsHtml(path, d) {
    const fileFunctions = (d.xray || [])
      .filter(function (r) { return r.path === path; })
      .sort(function (a, b) {
        const ca = (typeof a.cognitive === 'number') ? a.cognitive : 0;
        const cb = (typeof b.cognitive === 'number') ? b.cognitive : 0;
        return cb - ca;
      });
    if (!fileFunctions.length) return '';
    var html = '<h4>Functions</h4><ul class="drawer-partners">';
    for (var fi = 0; fi < Math.min(fileFunctions.length, 8); fi++) {
      const f = fileFunctions[fi];
      html += '<li><code>' + escapeHtml(f.function || '(anonymous)') + '</code>' +
        ' — cognitive <b>' + fmtNumberFlex(f.cognitive, 0) + '</b>' +
        (typeof f.start_line === 'number' ? ' <span class="drawer-author">L' + f.start_line + '</span>' : '') +
        '</li>';
    }
    if (fileFunctions.length > 8) {
      html += '<li>… ' + (fileFunctions.length - 8) + ' more functions</li>';
    }
    html += '</ul>';
    return html;
  }

  // Overview: clone groups. If the file appears in any clone family, surface
  // the count so the user can cross-reference with the Clones color mode in
  // the hotspot circle-pack.
  function drawerClonesHtml(path, d) {
    const cloneRow = (d.clones || []).find(function (r) { return r.path === path; });
    if (!cloneRow || !(cloneRow.groups || cloneRow.group_count || cloneRow.clone_groups)) return '';
    const groupCount = cloneRow.groups || cloneRow.group_count || cloneRow.clone_groups;
    return '<h4>Clones</h4><dl>' +
      '<dt>Clone groups</dt><dd>' + fmtInt(groupCount) + '</dd>' +
      '</dl>';
  }

  // Overview: code health.
  function drawerCodeHealthHtml(path, d) {
    const ch = (d.code_health || []).find(function (r) { return r.path === path; });
    if (!ch) return '';
    var corpusPctCell;
    if (ch.corpus_percentile != null) {
      var pct = Math.round(ch.corpus_percentile * 100);
      corpusPctCell = pct + '%' + (ch.beyond_corpus ? '+' : '');
      // Wilson 95% interval, when present — the corpus pool's sampling
      // uncertainty on the percentile, appended to the same line.
      if (ch.corpus_percentile_ci_low != null && ch.corpus_percentile_ci_high != null) {
        corpusPctCell += ' [' + Math.round(ch.corpus_percentile_ci_low * 100) + '–' +
          Math.round(ch.corpus_percentile_ci_high * 100) + '%]';
      }
    } else {
      corpusPctCell = '—';
    }
    return '<h4>Code health</h4><dl>' +
      '<dt>Score</dt><dd>' + fmtNumberFlex(ch.score, 1) + '</dd>' +
      '<dt>Cognitive</dt><dd>' + fmtNumberFlex(ch.cognitive, 0) + '</dd>' +
      '<dt>Health band</dt><dd>' + (ch.band || '—') + '</dd>' +
      '<dt>Corpus percentile</dt><dd>' + corpusPctCell + '</dd>' +
      '</dl>';
  }

  // Overview: marginal-owner risk chip (ownership × health signal).
  function drawerMarginalOwnerRiskHtml(path, d) {
    const mor = (d.marginal_owner_risk || []).find(function (r) { return r.path === path; });
    if (!mor) return '';
    var morLabel = mor.risk === 'high' ? '⚠ High owner risk' : '⚠ Elevated owner risk';
    // Reuses ki-knowledge-loss-badge styling intentionally — same visual weight as the knowledge-loss chip.
    return '<div class="ki-knowledge-loss-badge" title="' + mor.note + '">' +
      morLabel + ' — top active share ' + fmtNumberFlex(mor.top_active_share, 2) +
      '</div>';
  }


