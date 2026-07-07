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


