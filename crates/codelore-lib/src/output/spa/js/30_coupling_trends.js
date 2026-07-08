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

    // Depth source:
    //   'files' (default) → no aggregation: top-30 file pairs
    //   integer 2-6      → collapse entities to N path segments,
    //                       re-aggregate pairs, then top-30
    // The user setting lives in `Alpine.store('layout').sankeyDepth`
    // and is persisted across reloads.
    const TOP_N = 30;
    const sankeyLayout = (window.Alpine && window.Alpine.store)
      ? window.Alpine.store('layout') : null;
    const userSankeyDepth = sankeyLayout ? sankeyLayout.sankeyDepth : 'files';

    function modulePathSeg(p, depth) {
      const parts = (p || '').split('/');
      if (parts.length <= depth) {
        const lastSlash = (p || '').lastIndexOf('/');
        return lastSlash < 0 ? (p || '') : p.slice(0, lastSlash);
      }
      return parts.slice(0, depth).join('/');
    }

    var workingRows;
    if (typeof userSankeyDepth === 'number') {
      // Aggregate file-pair coupling to module-pair coupling at the
      // chosen depth. Self-pairs (s === t after collapse) and
      // duplicate (s, t) edges are merged by summing the shared-revision
      // count (`shared`) and taking the max coupling strength (`degree`,
      // the co-change percentage). These are CouplingRow's actual fields.
      const aggregated = {};
      for (var i = 0; i < rows.length; i++) {
        const r = rows[i];
        const a = modulePathSeg(r.entity_a, userSankeyDepth);
        const b = modulePathSeg(r.entity_b, userSankeyDepth);
        if (!a || !b || a === b) continue;
        const key = a < b ? a + '\x00' + b : b + '\x00' + a;
        if (!aggregated[key]) {
          aggregated[key] = {
            entity_a: a < b ? a : b,
            entity_b: a < b ? b : a,
            shared: 0,
            degree: 0,
          };
        }
        aggregated[key].shared += (r.shared || 0);
        const strength = (typeof r.degree === 'number') ? r.degree : 0;
        if (strength > aggregated[key].degree) {
          aggregated[key].degree = strength;
        }
      }
      workingRows = Object.keys(aggregated).map(function (k) { return aggregated[k]; });
    } else {
      workingRows = rows;
    }

    const topRows = workingRows.slice()
      .sort(function (a, b) {
        const ca = (typeof a.degree === 'number') ? a.degree : 0;
        const cb = (typeof b.degree === 'number') ? b.degree : 0;
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
        value: r.shared || 0,
      };
    });
    const nodes = Array.from(nodeNames).map(function (name) {
      return { name: name };
    });

    setChartAriaLabel(container,
      'Change-coupling Sankey flow across ' + nodes.length + ' entities, ' +
      links.length + ' co-change links');

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
        label: { color: token('--label-on-dark'), fontSize: 11 },
      }],
    });

    chart.on('click', function (params) {
      if (params.dataType === 'node' && params.data && params.data.name) {
        // In 'files' mode the node name IS a full repo-relative path, so
        // broadcast the selection. In module-depth mode the name is a
        // truncated module prefix that no file-level subscriber matches —
        // open the drawer directly without polluting the selection bus.
        if (userSankeyDepth === 'files' && window._codeloreShowDetail) {
          window._codeloreShowDetail(params.data.name);
        } else {
          showFileDetailDrawer(params.data.name, data);
        }
      }
    });

    // Cross-widget selection: emphasise the selected file's node (and its
    // links) in the coupling sankey; a null selection downplays everything.
    window._codeloreRegisterSelectionListener('coupling', function (selectedPath) {
      chart.dispatchAction({ type: 'downplay' });
      if (!selectedPath) return;
      // Node names live in the current depth's name-space: full paths in
      // 'files' mode, truncated module prefixes in module-depth mode. Map
      // the bus's full path into that space or the module view no-ops.
      const nodeName = (typeof userSankeyDepth === 'number')
        ? modulePathSeg(selectedPath, userSankeyDepth)
        : selectedPath;
      chart.dispatchAction({ type: 'highlight', seriesIndex: 0, name: nodeName });
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

    // User-tunable Top-N. Backend sends up to 50 paths; the frontend
    // ranks by total revisions across all months and slices to
    // `Alpine.store('layout').trendsTopN` (default 10, 'all' = no
    // cap). The selector persists across reloads.
    const trendsLayout = (window.Alpine && window.Alpine.store)
      ? window.Alpine.store('layout') : null;
    const trendsTopN = trendsLayout ? trendsLayout.trendsTopN : 10;

    // Build {month -> {path -> score}} and a sorted month list.
    const months = Array.from(new Set(rows.map(function (r) { return r.month; }))).sort();
    const byMonth = {};
    const pathTotals = {};
    for (var i = 0; i < rows.length; i++) {
      const r = rows[i];
      if (!byMonth[r.month]) byMonth[r.month] = {};
      byMonth[r.month][r.path] = r.hotspot_score;
      pathTotals[r.path] = (pathTotals[r.path] || 0) + (r.hotspot_score || 0);
    }
    const allPaths = Object.keys(pathTotals)
      .sort(function (a, b) { return pathTotals[b] - pathTotals[a]; });
    const paths = (trendsTopN === 'all')
      ? allPaths
      : allPaths.slice(0, Number(trendsTopN));
    // One series per path. `emphasis.focus: 'series'` + explicit
    // `blur` dim non-hovered lines so the user can isolate one
    // trajectory in a busy chart — without `blur`, ECharts 6 leaves
    // the rest at full opacity and the hovered line gets visually
    // lost.
    const series = paths.map(function (p) {
      return {
        name: p,
        type: 'line',
        smooth: true,
        symbol: 'circle',
        symbolSize: 5,
        emphasis: { focus: 'series', lineStyle: { width: 3 } },
        blur: { lineStyle: { opacity: 0.15 } },
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
    // Disambiguate collisions: two paths that share head + tail segments
    // (e.g. `app/a/svc/main.py` and `app/b/svc/main.py`) abbreviate to the
    // same label. ECharts merges same-named series + legend entries, so one
    // legend toggle would flip several files and the tooltip's full-path
    // line would show the wrong (last-colliding) path. On collision, fall
    // back to the full path, which is always unique.
    const shortByLong = {};
    const longByShort = {};
    paths.forEach(function (p) {
      let label = shortPath(p);
      if (longByShort[label] !== undefined && longByShort[label] !== p) {
        label = p;
      }
      shortByLong[p] = label;
      longByShort[label] = p;
    });
    const legendData = paths.map(function (p) { return shortByLong[p]; });

    setChartAriaLabel(container,
      'Hotspot-score trend for ' + paths.length + ' files over ' +
      months.length + ' months');

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
      // Vertical right-side legend. A horizontal top legend
      // collides with the y-axis name and overlaps itself once
      // there are more than ~6 entries with long paths — even
      // with the path-abbreviation. Vertical scrolling on the
      // right scales cleanly from 5 → 50 series without
      // overlapping the chart area or the axis labels.
      //
      // `selector` adds ECharts' built-in "All" / "Inv" buttons
      // at the bottom of the legend — one click clears the entire
      // selection, another click inverts. Saves the user from
      // having to click every file off individually just to
      // isolate one trajectory.
      legend: {
        type: 'scroll',
        orient: 'vertical',
        right: 8,
        top: 8,
        bottom: 30,
        textStyle: { color: getCssVar('--fg-dim'), fontSize: 11 },
        pageTextStyle: { color: getCssVar('--fg-dim'), fontSize: 11 },
        data: legendData,
        itemGap: 6,
        pageButtonGap: 4,
        width: 220,
        // `title` is the button label — ECharts ships `inverse` as
        // "Inv" by default, which reads cryptic. "Swap" tells the
        // user exactly what happens: the on/off state of every
        // file flips. From the default all-on view, one click of
        // Swap turns everything off, then they click the single
        // file they want to keep.
        selector: [
          { type: 'all',     title: 'All' },
          { type: 'inverse', title: 'Swap' },
        ],
        selectorPosition: 'end',
        selectorButtonGap: 4,
        selectorLabel: {
          color: getCssVar('--fg-dim'),
          fontSize: 10,
          padding: [2, 6],
          borderColor: getCssVar('--border'),
          borderWidth: 1,
          borderRadius: 4,
        },
      },
      grid: { top: 16, left: 70, right: 248, bottom: 40 },
      xAxis: {
        type: 'category',
        data: months,
        axisLabel: { color: getCssVar('--fg-dim'), fontSize: 11 },
        axisLine: { lineStyle: { color: getCssVar('--border') } },
      },
      yAxis: {
        type: 'value',
        // `nameLocation: 'middle'` + `nameRotate: 90` puts the
        // axis name along the axis instead of above it, which
        // would collide with the vertical legend on the right —
        // the original top-positioned "revisions / month" sat
        // right where the legend's first row now wants to be.
        name: 'revisions / month',
        nameLocation: 'middle',
        nameRotate: 90,
        nameGap: 40,
        nameTextStyle: { color: getCssVar('--fg-dim'), fontSize: 11 },
        axisLabel: { color: getCssVar('--fg-dim'), fontSize: 11 },
        splitLine: { lineStyle: { color: getCssVar('--bg-elev-2') } },
      },
      series: series.map(function (s) {
        return Object.assign({}, s, { name: shortByLong[s.name] || shortPath(s.name) });
      }),
    });
    // Cross-widget selection sync: each series corresponds to one
    // path (the path array's indexing matches the series array),
    // so selecting a file dispatches `highlight` on that series
    // index. Empty selection downplays everything back to neutral.
    window._codeloreRegisterSelectionListener('trends', function (selectedPath) {
      // Downplay first: ECharts highlight is additive, so without this a
      // direct A→B selection (the drawer is non-modal, so switching files
      // without closing it is reachable) leaves A's series bold under B.
      chart.dispatchAction({ type: 'downplay' });
      if (!selectedPath) return;
      const idx = paths.indexOf(selectedPath);
      if (idx >= 0) {
        chart.dispatchAction({ type: 'highlight', seriesIndex: idx });
      }
    });
  }


  // ─── §11b Widget: Kamei Delivery-Risk Sparkline ──────────────────

  function renderKameiRiskSparkline(allRows) {
    const container = document.getElementById('widget-kamei-risk-body');
    if (!container) return;
    if (!allRows.length) {
      container.innerHTML = '<div class="empty">No Kamei JIT-SDP data — repo too small or kamei::enrich was not wired.</div>';
      return;
    }

    // User-tunable window. Backend now sends up to 100 most recent
    // non-merge commits; the SPA slices reactively per
    // `Alpine.store('layout').kameiWindow` (default 30, 'all' = no
    // cap). Rows are chronological; slicing from the tail keeps
    // the "most recent N" semantic.
    const kameiLayout = (window.Alpine && window.Alpine.store)
      ? window.Alpine.store('layout') : null;
    const kameiWindow = kameiLayout ? kameiLayout.kameiWindow : 30;
    const rows = (kameiWindow === 'all')
      ? allRows
      : allRows.slice(-Number(kameiWindow));

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

    var fixCount = 0;
    for (var fi = 0; fi < rows.length; fi++) {
      if (rows[fi].fix) fixCount++;
    }
    setChartAriaLabel(container,
      'Kamei delivery-risk per commit over ' + rows.length + ' recent commits, ' +
      fixCount + ' flagged as bug-fixes');

    const chart = mountEcharts(container);
    chart.setOption({
      tooltip: {
        trigger: 'item',
        // Attach the tooltip DOM to the OUTER widget section
        // (#widget-kamei-risk) instead of the inner chart body
        // (#widget-kamei-risk-body). Positioning is then relative
        // to the whole panel — top-right lands in the panel's
        // header area, well clear of every bar in the chart
        // beneath. Without `appendTo`, ECharts attaches the
        // tooltip to the chart body so `{ top: 4, right: 4 }`
        // landed somewhere over the bars instead of above them.
        appendTo: function (chartDom) {
          return chartDom.closest('section.widget') || document.body;
        },
        position: function () {
          return { top: 4, right: 4 };
        },
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
        // `emphasis: { disabled: true }` — ECharts 6 had a regression
        // on this exact combination (`focus: 'self'` + per-data
        // `itemStyle.color` on a `type: 'bar'` series): hovering the
        // bar dropped its rendered color, painting it with the chart
        // background → "the bar disappears." Disabling emphasis
        // entirely is the safe contract: no hover transform on the
        // bar, the tooltip carries the affordance. Same idiom on
        // parallel-coords below.
        emphasis: { disabled: true },
      }],
    });
  }


  // ─── §11b-2 Widget: Delivery card ───────────────────────────────
  //
  // Renders a "Delivery" information card below the Kamei sparkline.
  // Data sources:
  //   d.delivery_metrics  — percentile rows from `delivery-metrics`
  //   d.release_cadence   — rows + __summary__ from `release-cadence`
  //   d.delivery_friction — top-5 friction files from `delivery-friction`
  //
  // All three degrade gracefully: if the source array is absent or
  // empty that number / section is simply omitted. The card itself is
  // hidden when all three are absent.
  //
  // These are git-only proxies, NOT DORA metrics. The card carries a
  // disclaimer line to prevent misreading.

  function renderDeliveryCard(d) {
    const container = document.getElementById('widget-delivery-card-body');
    if (!container) return;

    const dm = d.delivery_metrics || [];
    const cadence = d.release_cadence || [];
    const friction = d.delivery_friction || [];

    // Check if any data at all.
    if (!dm.length && !cadence.length && !friction.length) {
      container.innerHTML = '<div class="empty">No delivery data — run with --include-merges and release tags matching --release-tag-glob.</div>';
      return;
    }

    function findMetric(name) {
      for (var i = 0; i < dm.length; i++) {
        if (dm[i].metric === name) return dm[i];
      }
      return null;
    }

    var rows = '';

    // Rework % (p50)
    var rework = findMetric('rework_pct');
    if (rework) {
      var rPct = typeof rework.p50 === 'number' ? rework.p50.toFixed(1) : '—';
      var rBand = rework.p50 < 9 ? 'green' : rework.p50 < 15 ? 'yellow' : 'red';
      rows += '<tr><td>Rework</td>' +
        '<td class="delivery-value cl-band-' + rBand + '">' + rPct + ' %</td>' +
        '<td class="delivery-caveat">' + escapeHtml(rework.caveat || '') + '</td></tr>';
    }

    // Branch duration p75
    var branch = findMetric('branch_duration_hours');
    if (branch) {
      var bVal = typeof branch.p75 === 'number' ? branch.p75.toFixed(0) + ' h' : '—';
      rows += '<tr><td>Branch p75</td><td class="delivery-value">' + bVal + '</td>' +
        '<td class="delivery-caveat">' + escapeHtml(branch.caveat || '') + '</td></tr>';
    }

    // Lead-time proxy p50
    var lead = findMetric('lead_proxy_hours');
    if (lead) {
      var lVal = typeof lead.p50 === 'number' ? lead.p50.toFixed(0) + ' h' : '—';
      rows += '<tr><td>Lead proxy p50</td><td class="delivery-value">' + lVal + '</td>' +
        '<td class="delivery-caveat">' + escapeHtml(lead.caveat || '') + '</td></tr>';
    }

    // Release cadence — from __summary__ row
    var summary = null;
    for (var ci = 0; ci < cadence.length; ci++) {
      if (cadence[ci].tag === '__summary__') { summary = cadence[ci]; break; }
    }
    if (summary && typeof summary.days_since_prev === 'number') {
      var cVal = summary.days_since_prev.toFixed(0) + ' d';
      var trend = summary.trend ? ' (' + escapeHtml(summary.trend) + ')' : '';
      rows += '<tr><td>Cadence median</td><td class="delivery-value">' + cVal + trend + '</td><td></td></tr>';
    }

    // "Where is friction" — top friction files drill line.
    var frictionHtml = '';
    if (friction.length > 0) {
      frictionHtml = '<div class="delivery-friction-header">Where is friction:</div>' +
        '<ol class="delivery-friction-list">';
      for (var fi = 0; fi < friction.length; fi++) {
        var f = friction[fi];
        frictionHtml += '<li>' + escapeHtml(f.path || '') + '</li>';
      }
      frictionHtml += '</ol>';
    }

    container.innerHTML =
      '<table class="delivery-table">' +
        '<tbody>' + rows + '</tbody>' +
      '</table>' +
      frictionHtml +
      '<div class="delivery-disclaimer">Git-only proxies — not DORA metrics.</div>';
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
    setChartAriaLabel(container,
      'Hotspot treemap of ' + top.length + ' files across ' +
      treeData.length + ' top-level directories, sized by revisions');
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
        // Semantic-zoom drill-down. `leafDepth: 2` collapses the tree
        // so the top-level directories render first; clicking a
        // directory drills into its file children with an ECharts-
        // internal morph animation. The native breadcrumb (top-left
        // by default) tracks the drill path and gives one-click
        // ascent back up the hierarchy — much cleaner than the prior
        // single-flat-view that buried every file in a 200-leaf grid.
        // Spec: Apache ECharts treemap-drill-down example
        // <https://echarts.apache.org/examples/en/editor.html?c=treemap-drill-down>.
        leafDepth: 2,
        breadcrumb: {
          show: true,
          top: 6,
          left: 6,
          itemStyle: {
            color: getCssVar('--bg-elev'),
            borderColor: getCssVar('--border'),
            textStyle: { color: getCssVar('--fg') },
          },
        },
        label: { show: true, color: token('--label-on-saturated'), fontSize: 11 },
        upperLabel: { show: true, height: 18, color: getCssVar('--fg-dim'), fontSize: 11 },
        // Per-depth styling: directory level (depth 1) carries a
        // thicker border + larger gap to read as a container; file
        // level (depth 2) tightens both so leaves pack densely. With
        // `leafDepth: 2` only depths 1 and 2 are ever rendered, so a
        // depth-3 levels entry would be dead config.
        // Progressive color saturation is left to ECharts' default
        // visualMin/visualMax behavior so the existing tooltip
        // color-coding (revisions × hotspot_score) survives.
        levels: [
          { itemStyle: { borderColor: getCssVar('--border'), borderWidth: 3, gapWidth: 3 } },
          { itemStyle: { borderColor: getCssVar('--border'), borderWidth: 2, gapWidth: 2 } },
        ],
      }],
    });
    chart.on('click', function (params) {
      const d = params && params.data;
      if (d && d.cognitive != null) {
        if (window._codeloreShowDetail) {
          window._codeloreShowDetail(d.name);
        } else {
          showFileDetailDrawer(d.name, data);
        }
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
    // User-tunable Top-N. Sorted by hotspot_score descending so the
    // polylines remain the highest-pressure files at any setting.
    // Stored in `Alpine.store('layout').parallelTopN` (default 20,
    // 'all' = no cap), persisted across reloads.
    const parallelLayout = (window.Alpine && window.Alpine.store)
      ? window.Alpine.store('layout') : null;
    const parallelTopN = parallelLayout ? parallelLayout.parallelTopN : 20;
    const sorted = rows.slice()
      .sort(function (a, b) {
        const sa = (typeof a.hotspot_score === 'number') ? a.hotspot_score : -Infinity;
        const sb = (typeof b.hotspot_score === 'number') ? b.hotspot_score : -Infinity;
        return sb - sa;
      });
    const top = (parallelTopN === 'all') ? sorted : sorted.slice(0, Number(parallelTopN));
    setChartAriaLabel(container,
      'Parallel-coordinates plot of ' + top.length + ' files across ' +
      'revisions, cognitive complexity, code health, hotspot score and MI rank');
    const chart = mountEcharts(container);
    // Build the polyline data once so the cross-widget selection listener can
    // re-style individual lines by mutating their per-item lineStyle (emphasis
    // is disabled on this series — see below — so highlight/downplay are inert).
    const parallelData = top.map(function (r) {
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
    });
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
        // Position function pins the tooltip to the top-right of
        // the chart instead of following the cursor — at-cursor
        // placement on a parallel-coords polyline puts the popup
        // directly over the line the user is reading, so it looks
        // like the line vanished. Top-right keeps the data visible
        // and the popup readable in one glance.
        position: function (point, params, dom, rect, size) {
          return [size.viewSize[0] - size.contentSize[0] - 12, 8];
        },
        confine: true,
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
        // `emphasis: { disabled: true }` — ECharts 6 had the same
        // hovered-item-disappears regression on this parallel
        // series as on the Kamei bar series above. Disable the
        // emphasis transform entirely; the tooltip carries all the
        // hover affordance we need, the polyline remains its normal
        // colour and width regardless of hover state.
        emphasis: { disabled: true },
        data: parallelData,
      }],
    });
    chart.on('click', function (params) {
      if (params && params.name) {
        // Route through `_codeloreShowDetail` so the click both
        // opens the drawer AND publishes the selection — direct
        // `showFileDetailDrawer` would only do the former.
        if (window._codeloreShowDetail) {
          window._codeloreShowDetail(params.name);
        } else {
          showFileDetailDrawer(params.name, data);
        }
      }
    });
    // Cross-widget selection: emphasis is disabled on this series (ECharts 6
    // hover-disappears regression), so highlight/downplay do nothing. Instead
    // re-style the lines directly — the selected file's polyline goes bold
    // info-blue while the rest fade; a null selection restores every line to
    // the default warning colour. Mutates parallelData in place + re-applies.
    const parallelPaths = top.map(function (r) { return r.path; });
    const parallelBase = { color: token('--color-warning'), width: 1, opacity: 0.6 };
    window._codeloreRegisterSelectionListener('parallel-coords', function (selectedPath) {
      const idx = selectedPath ? parallelPaths.indexOf(selectedPath) : -1;
      for (var i = 0; i < parallelData.length; i++) {
        if (idx < 0) {
          parallelData[i].lineStyle = parallelBase;
        } else if (i === idx) {
          parallelData[i].lineStyle = { color: token('--color-info'), width: 3, opacity: 1 };
        } else {
          parallelData[i].lineStyle = { color: token('--color-warning'), width: 1, opacity: 0.12 };
        }
      }
      chart.setOption({ series: [{ data: parallelData }] });
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
    setChartAriaLabel(container,
      'Cognitive-complexity distribution across ' + values.length +
      ' functions, median ' + Math.round(med) + ', ' + outliers.length + ' outliers');
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
    // Roll up each pair to module-level using the first N path
    // segments (e.g. depth=2 → `app/services/x.py` → `app/services`).
    // Different repos have different natural granularities — a Rust
    // workspace at the crate level may want depth 1-2; a Python web
    // app with deep `app/services/<feature>/...` trees needs 3+.
    // Fixed depth + infra filter often collapses to 2-3 nodes; we
    // deepen adaptively until the chord has enough structure to
    // read.
    function modulePath(p, depth) {
      const parts = (p || '').split('/');
      if (parts.length <= depth) {
        const lastSlash = (p || '').lastIndexOf('/');
        return lastSlash < 0 ? (p || '') : p.slice(0, lastSlash);
      }
      return parts.slice(0, depth).join('/');
    }
    // Drop infrastructure / configuration files from the chord.
    // Change-coupling captures co-modification, so lock files,
    // env files, and top-level docs cluster with every release
    // commit they were bumped in — they pollute the module
    // diagram with edges that don't represent code architecture.
    // Keep this list conservative; users who explicitly want
    // infra coupling can read the raw coupling table.
    function isInfrastructureFile(p) {
      if (!p) return false;
      // Lock files (uv.lock, package-lock.json, Cargo.lock, poetry.lock, yarn.lock, ...)
      if (/\.lock$/i.test(p) || /-lock\.json$/i.test(p)) return true;
      // Env files (.env, .env.example, .env.test, .env.local, ...)
      if (/(^|\/)\.env(\..+)?$/i.test(p)) return true;
      // Docs (top-level .md / .rst / .txt + entire docs/** tree)
      if (/^docs?\//i.test(p)) return true;
      if (/\.(md|rst|txt|adoc)$/i.test(p)) return true;
      // Build / dependency manifests at any depth
      if (/(^|\/)(pyproject|Cargo|package|composer|Gemfile|setup)\.(toml|json|yaml|yml)$/i.test(p)) return true;
      if (/(^|\/)(requirements[^/]*|setup)\.(txt|cfg|py)$/i.test(p)) return true;
      // CI / repo metadata
      if (/^\.github\//i.test(p) || /^\.gitlab/i.test(p)) return true;
      if (/^\.(gitignore|gitattributes|dockerignore|editorconfig|prettierrc|eslintrc)/i.test(p)) return true;
      // Version markers
      if (/(^|\/)VERSION$/.test(p) || /(^|\/)CHANGELOG(\.[^/]+)?$/i.test(p)) return true;
      return false;
    }
    // Depth source:
    //   'auto' (default) → adaptive loop, deepens until ≥ MIN_NODES
    //   integer 2-6     → user-fixed depth, no adaptation
    // The user setting lives in `Alpine.store('layout').chordDepth`
    // (persisted across reloads). When the store value changes, an
    // Alpine effect in template.html fires the rerenderer list,
    // which re-runs this function with the new setting.
    const MIN_NODES_FOR_USEFUL_CHORD = 6;
    const layout = (window.Alpine && window.Alpine.store)
      ? window.Alpine.store('layout') : null;
    const userChordDepth = layout ? layout.chordDepth : 'auto';
    function aggregateAt(depth) {
      const ee = {};
      const nn = {};
      for (var i = 0; i < rows.length; i++) {
        const r = rows[i];
        if (isInfrastructureFile(r.entity_a) || isInfrastructureFile(r.entity_b)) continue;
        const a = modulePath(r.entity_a, depth);
        const b = modulePath(r.entity_b, depth);
        if (!a || !b || a === b) continue;
        const key = a < b ? a + '\x00' + b : b + '\x00' + a;
        ee[key] = (ee[key] || 0) + (r.shared || 1);
        nn[a] = true;
        nn[b] = true;
      }
      return { edges: ee, nodeCount: Object.keys(nn).length };
    }
    var edges = {};
    if (typeof userChordDepth === 'number') {
      edges = aggregateAt(userChordDepth).edges;
    } else {
      for (var depth = 2; depth <= 6; depth++) {
        const result = aggregateAt(depth);
        edges = result.edges;
        if (result.nodeCount >= MIN_NODES_FOR_USEFUL_CHORD) break;
      }
    }
    const linkRows = Object.keys(edges).map(function (k) {
      const parts = k.split('\x00');
      return { source: parts[0], target: parts[1], value: edges[k] };
    });
    if (!linkRows.length) {
      container.innerHTML = '<div class="empty">All change-coupling stays inside a single 2-segment module after dropping infrastructure files (lock / env / docs / build manifests). See the raw <em>coupling</em> table for the full pair list.</div>';
      return;
    }
    const nodes = {};
    for (var ei = 0; ei < linkRows.length; ei++) {
      nodes[linkRows[ei].source] = true;
      nodes[linkRows[ei].target] = true;
    }
    // Colour each module by its top-level (first path segment) group so
    // clusters are visually distinct; the lexical sort already places
    // same-group modules on a contiguous arc. When every module shares one
    // top-level root (single-root repo → one group), fall back to one
    // category per module so modules stay individually distinguishable
    // rather than collapsing to a single colour.
    function chordTopGroup(name) {
      const slash = name.indexOf('/');
      return slash < 0 ? name : name.slice(0, slash);
    }
    const sortedNames = Object.keys(nodes).sort();
    const groupNames = [];
    const groupIndex = {};
    for (var gi = 0; gi < sortedNames.length; gi++) {
      const g = chordTopGroup(sortedNames[gi]);
      if (!(g in groupIndex)) { groupIndex[g] = groupNames.length; groupNames.push(g); }
    }
    const chordPerNode = groupNames.length < 2;
    const categories = chordPerNode
      ? sortedNames.map(function (n) { return { name: n }; })
      : groupNames.map(function (g) { return { name: g }; });
    const nodeArr = sortedNames.map(function (n, idx) {
      return { name: n, category: chordPerNode ? idx : groupIndex[chordTopGroup(n)] };
    });
    setChartAriaLabel(container,
      'Module change-coupling chord diagram, ' + nodeArr.length + ' modules and ' +
      linkRows.length + ' coupled pairs');
    const chart = mountEcharts(container);
    chart.setOption({
      tooltip: { trigger: 'item' },
      series: [{
        type: 'graph',
        layout: 'circular',
        circular: { rotateLabel: true },
        data: nodeArr,
        categories: categories,
        links: linkRows,
        roam: false,
        label: { show: true, color: getCssVar('--fg-dim'), fontSize: 10, position: 'right' },
        lineStyle: {
          color: 'source',
          opacity: 0.45,
          curveness: 0.45,
        },
        emphasis: { focus: 'adjacency', lineStyle: { width: 3 } },
      }],
    });
  }


