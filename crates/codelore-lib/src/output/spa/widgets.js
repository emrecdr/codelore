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

  // -----------------------------------------------------------------
  // Data load
  // -----------------------------------------------------------------
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

  // Detail drawer state — set up once, reused by every widget that
  // wants to surface per-file details.
  initDetailDrawer();
  // Registry of re-render callbacks. Each ECharts widget pushes its
  // re-render fn so the theme toggle can repaint all of them when
  // CSS variables change. (Theme uses CSS variables for axis / grid
  // colors; ECharts caches the *resolved* values at setOption time
  // so a CSS variable update alone doesn't refresh the chart.)
  window._codeloreRerenderers = [];
  // Theme toggle (light / dark) — preference persisted in localStorage.
  initThemeToggle();
  // Color-mode toggles for the hotspot circle-pack (cognitive / author / ai).
  initHotspotColorToggles();

  // -----------------------------------------------------------------
  // Widget 0: KPI tiles (at-a-glance KPIs)
  // -----------------------------------------------------------------
  renderKpiTiles(data);

  // -----------------------------------------------------------------
  // Widget K: knowledge islands (CodeLore differentiator vs CodeScene)
  // -----------------------------------------------------------------
  renderKnowledgeIslands(data.knowledge_islands || []);

  // -----------------------------------------------------------------
  // Widget 1: hotspot circle-pack (the signature CodeScene view)
  // -----------------------------------------------------------------
  let currentHotspotColorMode = 'cognitive';
  renderHotspotCirclePack(data.hotspots || [], currentHotspotColorMode);
  window._codeloreRerenderers.push(function () {
    renderHotspotCirclePack(data.hotspots || [], currentHotspotColorMode);
  });

  // -----------------------------------------------------------------
  // Widget 2: hotspot table — sortable drill-down view of widget 1
  // -----------------------------------------------------------------
  renderHotspotTable(data.hotspots || []);

  // -----------------------------------------------------------------
  // Widget C: change-coupling sankey (top-N coupled file pairs)
  // -----------------------------------------------------------------
  renderCouplingSankey(data.coupling || []);
  window._codeloreRerenderers.push(function () {
    renderCouplingSankey(data.coupling || []);
  });

  // -----------------------------------------------------------------
  // v0.4.2 widgets (registered for theme re-render)
  // -----------------------------------------------------------------
  renderTrends(data.trends || []);
  window._codeloreRerenderers.push(function () { renderTrends(data.trends || []); });
  renderCalendarHeatmap(data.daily_commits || []);
  window._codeloreRerenderers.push(function () { renderCalendarHeatmap(data.daily_commits || []); });
  renderXRaySunburst(data.xray || []);
  window._codeloreRerenderers.push(function () { renderXRaySunburst(data.xray || []); });

  // Per-metric provenance definitions: formula in plain English + a
  // link to the research-foundations.md section that grounds the
  // metric. Surfaced as `?` tooltips on KPI tiles and table column
  // headers. Static data — no per-repo variation — so it lives in
  // the JS const map rather than the SpaDashboard JSON payload.
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
    const prior = echarts.getInstanceByDom(container);
    if (prior) prior.dispose();
    const chart = echarts.init(container, null, { renderer: 'canvas' });
    const nodes = root.descendants();
    const maxCognitive = nodes.reduce(function (acc, n) {
      const cog = n.data.metrics ? n.data.metrics.cognitive : 0;
      return Math.max(acc, cog);
    }, 0) || 1;

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
        renderItem: function (params, api) {
          const datum = api.value('_raw');
          if (!datum) return null;
          return {
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
          };
        },
        data: nodes
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
            } else {
              leafColor = heatmapColor(ratio);
            }
            const color = isLeaf
              ? leafColor
              : 'rgba(255, 255, 255, 0.02)';
            const stroke = isLeaf
              ? 'rgba(0, 0, 0, 0.3)'
              : 'rgba(255, 255, 255, 0.15)';
            return {
              value: [n.x, n.y],
              _raw: { x: n.x, y: n.y, r: n.r, color: color, stroke: stroke, opacity: isLeaf ? 0.85 : 1 },
              name: n.data.name || 'root',
              fullPath: n.data.fullPath || '',
              metrics: m || null,
              depth: n.depth,
              leafCount: n.leaves ? n.leaves().length : 0,
            };
          }),
      }],
    });

    chart.on('click', function (params) {
      const d = params && params.data;
      if (d && d.fullPath && d.metrics) {
        showFileDetailDrawer(d.fullPath, data);
      }
    });

    bindChartResize(chart, container);
  }

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
      let html = '<table><thead><tr>';
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
        // MI cell: number + band emoji when mi_rank is finite. Empty
        // when language unsupported by codelore-rca.
        let miCell = '';
        if (typeof r.mi === 'number' && isFinite(r.mi)) {
          let bandEmoji = '';
          if (typeof r.mi_rank === 'number' && isFinite(r.mi_rank)) {
            bandEmoji = r.mi_rank >= 0.75 ? ' 🟢'
              : r.mi_rank >= 0.25 ? ' 🟡' : ' 🔴';
          }
          miCell = r.mi.toFixed(1) + bandEmoji;
        }
        // AI cell: percentage rendered as X% (rounded — table is dense,
        // decimal point would crowd).
        const aiCell = (typeof r.ai_pct === 'number' && isFinite(r.ai_pct))
          ? Math.round(r.ai_pct) + '%'
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
      showNext.textContent = 'Show next ' + next;
      showNext.addEventListener('click', function () { renderNextPage(PAGE_SIZE); });
      actionsEl.appendChild(showNext);
      if (more > PAGE_SIZE) {
        const showAll = document.createElement('button');
        showAll.type = 'button';
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
      if (debounceTimer) clearTimeout(debounceTimer);
      debounceTimer = setTimeout(rerender, 80);
    });

    // Initial render.
    rerender();
  }

  // -----------------------------------------------------------------
  // Widget 0: KPI tiles — at-a-glance metrics
  // -----------------------------------------------------------------
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
      html += '<div class="kpi-tile">' +
        '<div class="kpi-label">' + escapeHtml(t.label) + tip + '</div>' +
        '<div class="kpi-value">' + escapeHtml(t.value) + '</div>' +
        '<div class="kpi-sub">' + escapeHtml(t.sub) + '</div>' +
        '</div>';
    }
    container.innerHTML = html;
  }

  // -----------------------------------------------------------------
  // Widget C: change-coupling sankey
  // -----------------------------------------------------------------
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

    const prior = echarts.getInstanceByDom(container);
    if (prior) prior.dispose();
    const chart = echarts.init(container, null, { renderer: 'canvas' });
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

    bindChartResize(chart, container);
  }

  // -----------------------------------------------------------------
  // Widget K: knowledge islands (the CodeLore differentiator)
  // -----------------------------------------------------------------
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

  // -----------------------------------------------------------------
  // Detail drawer (cross-widget click target)
  // -----------------------------------------------------------------
  function initDetailDrawer() {
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

    body.innerHTML = html;
    drawer.hidden = false;
  }

  // Expose so the hotspot table can call it on row click.
  window._codeloreShowDetail = function (path) { showFileDetailDrawer(path, data); };

  // -----------------------------------------------------------------
  // W9: trends multi-line
  // -----------------------------------------------------------------
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

    const prior = echarts.getInstanceByDom(container);
    if (prior) prior.dispose();
    const chart = echarts.init(container, null, { renderer: 'canvas' });
    chart.setOption({
      tooltip: { trigger: 'axis' },
      legend: {
        top: 0,
        type: 'scroll',
        textStyle: { color: getCssVar('--fg-dim'), fontSize: 11 },
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
      series: series,
    });
    bindChartResize(chart, container);
  }

  // -----------------------------------------------------------------
  // W10: calendar heatmap of commits per day
  // -----------------------------------------------------------------
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

    const prior = echarts.getInstanceByDom(container);
    if (prior) prior.dispose();
    const chart = echarts.init(container, null, { renderer: 'canvas' });
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
    bindChartResize(chart, container);
  }

  // -----------------------------------------------------------------
  // W8: X-Ray sunburst — function-level complexity drill-down
  // -----------------------------------------------------------------
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
    const root = { name: 'all', children: [] };
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
      fileNode.children.push({
        name: r.function || '<anonymous>',
        value: r.cognitive,
        cognitive: r.cognitive,
        startLine: r.start_line,
        endLine: r.end_line,
        fullPath: r.path,
      });
    }

    const prior = echarts.getInstanceByDom(container);
    if (prior) prior.dispose();
    const chart = echarts.init(container, null, { renderer: 'canvas' });
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
        levels: [
          {},
          { itemStyle: { color: '#2b5d39' }, label: { color: '#fff', fontSize: 11 } },
          { itemStyle: { color: '#3d7d4f' }, label: { color: '#fff', fontSize: 10 } },
          { itemStyle: { color: '#5fa472' }, label: { color: '#fff', fontSize: 9 } },
        ],
      }],
    });

    chart.on('click', function (params) {
      const d = params && params.data;
      if (d && d.fullPath && d.cognitive != null) {
        showFileDetailDrawer(d.fullPath, data);
      }
    });

    bindChartResize(chart, container);
  }

  // -----------------------------------------------------------------
  // W7 + W11: color-mode toggles on the hotspot circle-pack
  // -----------------------------------------------------------------
  function initHotspotColorToggles() {
    const bar = document.getElementById('hotspot-color-toggles');
    if (!bar) return;
    const buttons = bar.querySelectorAll('button.toggle');
    for (var i = 0; i < buttons.length; i++) {
      buttons[i].addEventListener('click', function (evt) {
        const mode = evt.currentTarget.getAttribute('data-mode');
        // Update active state on buttons
        for (var j = 0; j < buttons.length; j++) {
          buttons[j].classList.toggle('active', buttons[j] === evt.currentTarget);
        }
        // Re-render with new color mode. Update the shared cursor so
        // the theme-toggle re-render uses the active mode too.
        currentHotspotColorMode = mode;
        renderHotspotCirclePack(data.hotspots || [], mode);
      });
    }
  }

  // -----------------------------------------------------------------
  // Theme toggle (light / dark)
  // -----------------------------------------------------------------
  function initThemeToggle() {
    const btn = document.getElementById('theme-toggle');
    const label = document.getElementById('theme-toggle-label');
    if (!btn || !label) return;
    const STORAGE_KEY = 'codelore-theme';
    function apply(theme) {
      document.documentElement.setAttribute('data-theme', theme);
      label.textContent = theme === 'light' ? 'Dark mode' : 'Light mode';
    }
    // Restore stored preference (default: dark, matching the original look)
    let stored = 'dark';
    try { stored = localStorage.getItem(STORAGE_KEY) || 'dark'; } catch (e) {}
    apply(stored);
    btn.addEventListener('click', function () {
      const next = document.documentElement.getAttribute('data-theme') === 'light' ? 'dark' : 'light';
      apply(next);
      try { localStorage.setItem(STORAGE_KEY, next); } catch (e) {}
      // Re-render every ECharts widget so axis labels / grids /
      // gradient colors pick up the new CSS variable values
      // (ECharts caches resolved colors at setOption time).
      const rerenderers = window._codeloreRerenderers || [];
      for (var i = 0; i < rerenderers.length; i++) {
        try { rerenderers[i](); } catch (e) { console.warn('rerender failed:', e); }
      }
    });
  }

  // -----------------------------------------------------------------
  // Helpers
  // -----------------------------------------------------------------

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
})();
