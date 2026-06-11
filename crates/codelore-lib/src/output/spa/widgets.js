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
  renderHotspotCirclePack(data.hotspots || []);

  // -----------------------------------------------------------------
  // Widget 2: hotspot table — sortable drill-down view of widget 1
  // -----------------------------------------------------------------
  renderHotspotTable(data.hotspots || []);

  // -----------------------------------------------------------------
  // Widget C: change-coupling sankey (top-N coupled file pairs)
  // -----------------------------------------------------------------
  renderCouplingSankey(data.coupling || []);

  function renderHotspotCirclePack(rows) {
    const container = document.getElementById('widget-hotspot-circle-pack-body');
    if (!container) return;
    if (!rows.length) {
      container.innerHTML = '<div class="empty">No hotspots to display. ' +
        'The repository may be too small, or thresholds filtered everything out.</div>';
      return;
    }

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
            const color = isLeaf
              ? heatmapColor(ratio)
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

    window.addEventListener('resize', function () { chart.resize(); });
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
      { key: 'revisions',     label: 'Revisions',    cls: 'num',  kind: 'number', defaultDir: -1 },
      { key: 'cognitive',     label: 'Cognitive',    cls: 'num',  kind: 'number', defaultDir: -1 },
      { key: 'code_health',   label: 'Code Health',  cls: 'num',  kind: 'number', defaultDir: 1 },
      { key: 'hotspot_score', label: 'Hotspot Score', cls: 'num', kind: 'number', defaultDir: -1 },
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
        html += '<th class="' + (active ? 'active' : '') + '"' +
          ' data-key="' + escapeHtml(c.key) + '">' +
          escapeHtml(c.label) +
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
        html += '<tr data-path="' + escapeHtml(r.path) + '" class="hotspot-row" style="cursor:pointer">' +
          '<td class="path">' + escapeHtml(r.path) + '</td>' +
          '<td class="num">' + (r.revisions != null ? r.revisions : '') + '</td>' +
          '<td class="num">' + fmtNumber(r.cognitive, { decimals: 0 }) + '</td>' +
          '<td class="num">' + fmtNumber(r.code_health, { decimals: 1 }) + '</td>' +
          '<td class="num">' + fmtNumber(r.hotspot_score, { decimals: 2 }) + '</td>' +
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
      value: fmtInt(fileCount),
      sub: fileCount === 1 ? 'one file' : 'live at HEAD',
    });

    // Tile 2: commits
    const commits = summaryByName.commits || summaryByName['number-of-commits'] || 0;
    tiles.push({
      label: 'Commits',
      value: fmtInt(commits),
      sub: 'in the analysed history',
    });

    // Tile 3: authors
    const authors = summaryByName['authors'] || summaryByName['number-of-authors'] || 0;
    tiles.push({
      label: 'Distinct authors',
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
        value: fmtInt(knowledgeIslands.length),
        sub: 'departed-author files',
      });
    }

    // Tile 7: coupling pair count
    if (coupling.length) {
      tiles.push({
        label: 'Coupled file pairs',
        value: fmtInt(coupling.length),
        sub: 'Fisher-significant',
      });
    }

    var html = '';
    for (var j = 0; j < tiles.length; j++) {
      const t = tiles[j];
      html += '<div class="kpi-tile">' +
        '<div class="kpi-label">' + escapeHtml(t.label) + '</div>' +
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

    window.addEventListener('resize', function () { chart.resize(); });
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
