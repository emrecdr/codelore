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
      '<th scope="col">Path</th>' +
      '<th scope="col">Departed author</th>' +
      '<th scope="col" class="num">Ownership %</th>' +
      '<th scope="col" class="num">Days since active</th>' +
      '<th scope="col" class="num">LOC</th>' +
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
  //
  // Each tile also doubles as a jump link to its dashboard section
  // (looked up by `name` — see FACTOR_TILE_TARGETS below — rather than
  // by array index, because any subset of the four can be absent when
  // the underlying data, e.g. health-trend or delivery metrics, is
  // missing). Clicking or pressing Enter on a mapped tile scrolls to
  // its section via `scrollToDashSection` (00_setup_boot.js) — the same
  // path the sticky nav chips use, so `location.hash` is never touched.
  function renderFactorHeader(factors, opts) {
    const container = document.getElementById('widget-factor-header-body');
    if (!container) return;
    if (!factors || !factors.length) {
      container.innerHTML = '<div class="empty">No factor data — run with health-trend enabled.</div>';
      return;
    }

    // Declared inside the function (not at module scope) so its value is
    // guaranteed to exist by the time it's read: `renderFactorHeader` is
    // the FIRST widget in `WIDGETS` (00_setup_boot.js) and runs
    // synchronously, before this file's own top-level statements — a
    // module-scope `var` here would still be `undefined` at that point
    // (only the declaration hoists, not the assignment).
    const FACTOR_TILE_TARGETS = {
      Code: 'group-code-health',
      Architecture: 'group-architecture',
      Knowledge: 'group-knowledge',
      Delivery: 'group-delivery',
    };

    const o = opts || {};
    const greenMin = typeof o.health_green_min === 'number' ? o.health_green_min : 70;
    const yellowMin = typeof o.health_yellow_min === 'number' ? o.health_yellow_min : 40;

    function scoreColor(score) {
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
      const color = scoreColor(tile.headline);
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
      var reworkColor = REWORK_BAND_COLORS[tile.band] || '';
      var html = '<div class="factor-numbers">';
      var nums = tile.numbers || [];
      for (var ni = 0; ni < nums.length; ni++) {
        var pair = nums[ni];
        var label = escapeHtml(pair[0] || '');
        var value = escapeHtml(pair[1] || '');
        // Only the first number (rework %) gets band coloring.
        var valStyle = (ni === 0 && reworkColor) ? ' style="color:' + reworkColor + ';"' : '';
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
      const jumpTarget = FACTOR_TILE_TARGETS[t.name];
      const jumpAttrs = jumpTarget
        ? ' data-target="' + jumpTarget + '" role="link" tabindex="0"'
        : '';
      html += '<div class="factor-tile" id="factor-tile-' + i + '"' + jumpAttrs + '>' +
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

    // Wire the jump-link tiles (only those with a mapped `data-target`;
    // see FACTOR_TILE_TARGETS above) onto the same scroll path the
    // sticky nav chips use — never `location.hash`. `role="link"` +
    // `tabindex="0"` in the markup above give the tile a natural tab
    // stop; Enter activates it here, matching native `<a>` semantics
    // (Space is intentionally not treated as activation).
    const jumpTiles = container.querySelectorAll('.factor-tile[data-target]');
    for (let ji = 0; ji < jumpTiles.length; ji++) {
      const tileEl = jumpTiles[ji];
      const target = tileEl.getAttribute('data-target');
      tileEl.addEventListener('click', function () { scrollToDashSection(target); });
      tileEl.addEventListener('keydown', function (evt) {
        if (evt.key === 'Enter') {
          evt.preventDefault();
          scrollToDashSection(target);
        }
      });
    }

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
              lineStyle: { width: 1.5, color: scoreColor(tile.headline) },
            }],
          });
        } catch (e) {
          console.error('codelore: factor sparkline render failed for', tile.name, e);
        }
      })(j, factors[j]);
    }
  }

