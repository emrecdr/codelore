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

  // -----------------------------------------------------------------
  // Widget 1: hotspot circle-pack (the signature CodeScene view)
  // -----------------------------------------------------------------
  renderHotspotCirclePack(data.hotspots || []);

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

    window.addEventListener('resize', function () { chart.resize(); });
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

  function escapeHtml(s) {
    return String(s || '')
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;')
      .replace(/"/g, '&quot;')
      .replace(/'/g, '&#39;');
  }
})();
