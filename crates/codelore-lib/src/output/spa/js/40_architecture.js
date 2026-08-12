  // ─── §11g Architecture widgets: shared module roll-up ────────────
  // Both the force graph and the dependency-structure matrix roll file
  // paths up to their module at `depth` segments and aggregate the
  // resolved import edges the same way; the shared helpers live here so
  // the two views can never drift.

  // Roll a file path up to its module at `depth` path segments (parent
  // dir when the file has fewer than `depth` segments, so leaf-level
  // imports don't collapse onto themselves).
  function modulePath(p, depth) {
    const parts = (p || '').split('/');
    if (parts.length <= depth) {
      const lastSlash = (p || '').lastIndexOf('/');
      return lastSlash < 0 ? (p || '') : p.slice(0, lastSlash);
    }
    return parts.slice(0, depth).join('/');
  }

  // Aggregate resolved import edges to module granularity at `depth`,
  // dropping self-edges. Returns `{ edges: {src\x00tgt: count}, nodes }`.
  function aggregateImportsAt(imports, depth) {
    const ee = {};
    const nn = {};
    for (var i = 0; i < imports.length; i++) {
      const imp = imports[i];
      if (!imp.target_path) continue;
      const s = modulePath(imp.src_path, depth);
      const t = modulePath(imp.target_path, depth);
      if (!s || !t || s === t) continue;
      const key = s + '\x00' + t;
      ee[key] = (ee[key] || 0) + 1;
      nn[s] = true;
      nn[t] = true;
    }
    return { edges: ee, nodes: nn };
  }

  // Aggregate change-coupling rows to module granularity at `depth`,
  // dropping self-pairs (both files rolling up to the same module). A
  // module pair's weight is the MAX `degree` among the file-pairs that
  // rolled up into it — the strongest observed signal survives the
  // roll-up rather than being diluted by an average. Keys are
  // canonical (alphabetically smaller module first) so a lookup never
  // needs to know which side of the pair it holds. Feeds the DSM
  // Fusion cell-mode (`classifyCells`) the same way `aggregateImportsAt`
  // feeds the structural cells.
  function aggregateCouplingAt(coupling, depth) {
    const cc = {};
    for (var i = 0; i < coupling.length; i++) {
      const row = coupling[i];
      const a = modulePath(row.entity_a, depth);
      const b = modulePath(row.entity_b, depth);
      if (!a || !b || a === b) continue;
      const key = (a < b) ? (a + '\x00' + b) : (b + '\x00' + a);
      const deg = (typeof row.degree === 'number') ? row.degree : 0;
      if (!(key in cc) || deg > cc[key]) cc[key] = deg;
    }
    return cc;
  }

  // ─── §11g Widget: Architecture force-graph ───────────────────────

  function renderArchGraph(imports, violations, unstable, roles) {
    violations = violations || [];
    unstable = unstable || [];
    roles = roles || [];
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
    // Depth source:
    //   'auto' (default) → adaptive loop, deepens until ≥ MIN_NODES
    //   integer 2-6     → user-fixed depth, no adaptation
    // The user setting lives in `Alpine.store('layout').archGraphDepth`
    // (persisted across reloads). When the store value changes, an
    // Alpine effect in template.html fires the rerenderer list,
    // which re-runs this function with the new setting.
    const MIN_NODES_FOR_USEFUL_GRAPH = 8;
    const archLayout = (window.Alpine && window.Alpine.store)
      ? window.Alpine.store('layout') : null;
    const userArchDepth = archLayout ? archLayout.archGraphDepth : 'auto';
    var edges = {};
    var nodes = {};
    // The depth the structural aggregation settled on — reused for the
    // fusion overlay so violation edges + unstable nodes roll up to the
    // SAME module granularity as the import edges.
    var chosenDepth = (typeof userArchDepth === 'number') ? userArchDepth : 6;
    if (typeof userArchDepth === 'number') {
      const result = aggregateImportsAt(imports, userArchDepth);
      edges = result.edges;
      nodes = result.nodes;
    } else {
      for (var depth = 2; depth <= 6; depth++) {
        const result = aggregateImportsAt(imports, depth);
        edges = result.edges;
        nodes = result.nodes;
        chosenDepth = depth;
        if (Object.keys(nodes).length >= MIN_NODES_FOR_USEFUL_GRAPH) break;
      }
    }
    // Fusion overlay #1 — modularity-violation edges: Fisher-significant
    // co-change pairs with NO structural import edge, aggregated to
    // module level. These are the "temporal-only" edges an import graph
    // cannot show (implicit/hidden coupling). Keyed strongest-degree.
    const violEdges = {};
    for (var vi = 0; vi < violations.length; vi++) {
      const v = violations[vi];
      const vs = modulePath(v.entity_a, chosenDepth);
      const vt = modulePath(v.entity_b, chosenDepth);
      if (!vs || !vt || vs === vt) continue;
      const vkey = vs + '\x00' + vt;
      const vd = (typeof v.degree === 'number') ? v.degree : 0;
      if (!(vkey in violEdges) || vd > violEdges[vkey]) violEdges[vkey] = vd;
      nodes[vs] = true;
      nodes[vt] = true;
    }
    // Fusion overlay #2 — unstable-interface modules (warning nodes):
    // heavily-imported files that change often and drag their dependents.
    const unstableModules = {};
    for (var ui = 0; ui < unstable.length; ui++) {
      const um = modulePath(unstable[ui].path, chosenDepth);
      if (!um) continue;
      unstableModules[um] = (unstableModules[um] || 0) + 1;
      nodes[um] = true;
    }
    // Architecture roles (Core/Shared/Control/Periphery) + cycle
    // membership, aggregated to module level. A module takes the
    // highest-precedence role among its files (core > control > shared >
    // periphery). Roles only COLOUR existing nodes — they don't add
    // isolated single-file nodes. Also accumulate the propagation cost.
    const ROLE_RANK = { core: 3, control: 2, shared: 1, periphery: 0 };
    const moduleRole = {};
    const moduleInCycle = {};
    // Shallowest topological level among a module's files — the band a
    // module sits in for the layered layout (entry points = level 0).
    const moduleLevel = {};
    var vfoSum = 0;
    var fileCount = 0;
    var filesInCycles = 0;
    for (var rri = 0; rri < roles.length; rri++) {
      const rr = roles[rri];
      fileCount += 1;
      vfoSum += (typeof rr.vfo === 'number') ? rr.vfo : 0;
      if (rr.in_cycle) filesInCycles += 1;
      const rm = modulePath(rr.path, chosenDepth);
      if (!rm) continue;
      const cur = moduleRole[rm];
      if (cur === undefined || (ROLE_RANK[rr.role] || 0) > (ROLE_RANK[cur] || 0)) {
        moduleRole[rm] = rr.role;
      }
      if (rr.in_cycle) moduleInCycle[rm] = true;
      const lv = (typeof rr.level === 'number') ? rr.level : 1e9;
      if (moduleLevel[rm] === undefined || lv < moduleLevel[rm]) moduleLevel[rm] = lv;
    }
    // Propagation cost = mean(vfo)/fileCount = sum(vfo)/fileCount² — "a
    // change to a random file reaches this fraction of the system".
    const propagationCost = fileCount > 0 ? (vfoSum / (fileCount * fileCount)) : 0;
    if (!Object.keys(nodes).length) {
      container.innerHTML = '<div class="empty">All resolved imports stay intra-module — no inter-module edges to graph.</div>';
      return;
    }
    const roleColors = {
      core: token('--color-error') || '#dc2626',
      control: token('--color-warning') || '#d97706',
      shared: token('--color-info') || '#2563eb',
      periphery: token('--fg-dim') || '#6b7280',
    };
    const violColor = roleColors.control;
    const cycleRing = getCssVar('--fg') || '#111827';
    const ROLE_ORDER = ['core', 'control', 'shared', 'periphery'];
    const nodeArr = Object.keys(nodes).map(function (n) {
      const role = moduleRole[n] || 'periphery';
      const isUnstable = !!unstableModules[n];
      const inCycle = !!moduleInCycle[n];
      const cat = ROLE_ORDER.indexOf(role);
      return {
        name: n,
        symbol: isUnstable ? 'diamond' : 'circle',
        symbolSize: isUnstable ? 42 : 30,
        category: cat < 0 ? 3 : cat,
        // A high-contrast ring marks modules that sit in a dependency
        // cycle; the fill stays the role colour.
        itemStyle: inCycle ? { borderColor: cycleRing, borderWidth: 3 } : undefined,
      };
    });
    // Layout mode: 'force' (default physics) or 'layered' (topological
    // bands). Layered de-hairballs the graph — modules stack in
    // horizontal bands by level, so forward deps flow downward and
    // back-edges (cycles) visibly run back up. Encodings (role colour,
    // cycle ring, unstable diamond) are on the nodes, so they carry over.
    const layoutMode = (archLayout && archLayout.archGraphLayout === 'layered')
      ? 'layered' : 'force';
    if (layoutMode === 'layered') {
      const W = container.clientWidth || 900;
      const padX = 64;
      const padTop = 40;
      const padBot = 40;
      const rowGap = 96;
      const minSpacingX = 96; // horizontal room per node so labels don't collide
      const usableW = W - 2 * padX;
      const perRow = Math.max(1, Math.floor(usableW / minSpacingX));
      // Bucket nodes by level in one pass; the distinct levels (shallow→
      // deep, entry points first) fall out of the bucket keys.
      const byLevel = {};
      nodeArr.forEach(function (nd) {
        const lv = (moduleLevel[nd.name] === undefined) ? 0 : moduleLevel[nd.name];
        (byLevel[lv] = byLevel[lv] || []).push(nd);
      });
      const levelsPresent = Object.keys(byLevel)
        .map(Number)
        .sort(function (a, b) { return a - b; });
      // Each level occupies one or more sub-rows: a band wider than
      // `perRow` wraps instead of squeezing its nodes (and their labels)
      // into an unreadable single line. Rows accumulate top-to-bottom so
      // level order — and the downward forward-dep flow — is preserved.
      var rowCursor = 0;
      levelsPresent.forEach(function (lv) {
        const band = byLevel[lv].sort(function (a, c) { return a.name < c.name ? -1 : 1; });
        const subRows = Math.max(1, Math.ceil(band.length / perRow));
        for (var sr = 0; sr < subRows; sr++) {
          const slice = band.slice(sr * perRow, (sr + 1) * perRow);
          const k = slice.length;
          const yRow = padTop + (rowCursor + sr) * rowGap;
          slice.forEach(function (nd, i) {
            nd.x = (k === 1) ? (W / 2) : (padX + (i / (k - 1)) * usableW);
            nd.y = yRow;
          });
        }
        rowCursor += subRows;
      });
      // Grow the container to fit all rows (force mode keeps the compact
      // default); panning via roam handles anything still off-screen.
      const totalRows = Math.max(rowCursor, 1);
      container.style.height = (padTop + padBot + (totalRows - 1) * rowGap + 40) + 'px';
    } else {
      // Restore the template's default height when leaving layered mode.
      container.style.height = '380px';
    }
    const structuralLinks = Object.keys(edges).map(function (k) {
      const parts = k.split('\x00');
      return { source: parts[0], target: parts[1], value: edges[k], _kind: 'import' };
    });
    const violationLinks = Object.keys(violEdges).map(function (k) {
      const parts = k.split('\x00');
      return {
        source: parts[0],
        target: parts[1],
        value: violEdges[k],
        _kind: 'violation',
        lineStyle: { color: violColor, type: 'dashed', opacity: 0.9, width: 2, curveness: 0.2 },
      };
    });
    const edgeArr = structuralLinks.concat(violationLinks);
    const cycleModuleCount = Object.keys(moduleInCycle).length;
    setChartAriaLabel(container,
      'Architecture graph, ' + nodeArr.length + ' modules coloured by role ' +
      '(core/control/shared/periphery), ' + structuralLinks.length + ' import edges, ' +
      violationLinks.length + ' dashed modularity-violation edges, ' +
      Object.keys(unstableModules).length + ' unstable-interface modules shown as diamonds, ' +
      cycleModuleCount + ' modules in dependency cycles shown ringed. Propagation cost ' +
      (propagationCost * 100).toFixed(1) + ' percent.');
    const chart = mountEcharts(container);
    const titleText = fileCount > 0
      ? 'Propagation cost ' + (propagationCost * 100).toFixed(1) + '%  ·  ' +
        filesInCycles + ' files in cycles'
      : '';
    chart.setOption({
      title: {
        text: titleText,
        left: 'center',
        top: 4,
        textStyle: { color: getCssVar('--fg-dim'), fontSize: 12, fontWeight: 'normal' },
      },
      tooltip: {
        trigger: 'item',
        formatter: function (p) {
          if (p.dataType === 'edge') {
            if (p.data && p.data._kind === 'violation') {
              return 'Modularity violation — co-change, no import<br/>' +
                escapeHtml(p.data.source) + ' &harr; ' + escapeHtml(p.data.target) +
                '<br/>coupling degree ' + (Number(p.data.value) || 0).toFixed(1) + '%';
            }
            return 'Imports: ' + escapeHtml(p.data.source) + ' &rarr; ' + escapeHtml(p.data.target) +
              ' (' + (Number(p.data.value) || 0) + ')';
          }
          return escapeHtml(p.name) + '<br/>role: ' + (moduleRole[p.name] || 'periphery') +
            (moduleInCycle[p.name] ? ' &middot; in cycle' : '') +
            (unstableModules[p.name] ? ' &middot; unstable interface' : '');
        },
      },
      legend: [{
        data: ROLE_ORDER,
        textStyle: { color: getCssVar('--fg-dim') },
        bottom: 0,
      }],
      series: [{
        type: 'graph',
        layout: 'force',
        categories: [
          { name: 'core', itemStyle: { color: roleColors.core } },
          { name: 'control', itemStyle: { color: roleColors.control } },
          { name: 'shared', itemStyle: { color: roleColors.shared } },
          { name: 'periphery', itemStyle: { color: roleColors.periphery } },
        ],
        data: nodeArr,
        links: edgeArr,
        roam: true,
        layout: layoutMode === 'layered' ? 'none' : 'force',
        force: { repulsion: 200, edgeLength: 80 },
        // In layered mode draw arrowheads so the downward = forward,
        // upward = back-edge (cycle) flow is legible; force mode keeps
        // the cleaner undecorated lines.
        edgeSymbol: layoutMode === 'layered' ? ['none', 'arrow'] : 'none',
        edgeSymbolSize: 7,
        // Layered mode packs nodes tightly, so label with the leaf
        // segment only (full module path still shows in the tooltip);
        // force mode has room for the full name.
        label: {
          show: true,
          color: getCssVar('--fg-dim'),
          fontSize: 11,
          formatter: layoutMode === 'layered'
            ? function (p) { const s = String(p.name).split('/'); return s[s.length - 1]; }
            : undefined,
        },
        lineStyle: { color: token('--color-info'), opacity: 0.6, width: 1.5 },
        emphasis: { focus: 'adjacency', lineStyle: { width: 3 } },
      }],
    });
    // Reset-zoom for the arch graph: re-run the renderer to wipe
    // the chart's internal `roam` state (pan offset + zoom level)
    // — ECharts reuses the chart instance via getInstanceByDom, so
    // this is a cheap setOption, not a full re-mount. Same effect
    // as the Hotspots widget's `_codeloreZoomReset` but driven by
    // ECharts' built-in roam reset instead of the CSS-transform
    // overlay.
    window._codeloreResetZoomHandlers['widget-arch-graph'] = function () {
      renderArchGraph(imports, violations, unstable, roles);
    };
  }


  // ─── §11h Widget: Dependency Structure Matrix ────────────────────

  // The scalable, layer-ordered view of the same import graph. A DSM
  // (Steward 1981; Sangal et al. 2005) is the matrix form of the
  // dependency graph: it does not hairball as the module count grows.
  // Modules are ordered by architectural layer (from architecture-
  // roles' topological `level`); a cell (row imports col) coloured blue
  // is a healthy forward dependency (above the diagonal), red is a
  // back-edge (below the diagonal) — which only happens inside a
  // dependency cycle. A clean acyclic architecture is a triangular,
  // all-blue matrix.
  // ─── §11i Widget: Architecture decay trend ──────────────────────
  // Dual-axis line over the sampled historical revisions: propagation
  // cost (left, blue) and dependency-cycle count (right, red, stepped).
  // Answers "is the architecture decaying, and when did it start?" —
  // the HEAD metrics projected back across history.
  function renderArchTrend(rows) {
    const container = document.getElementById('widget-arch-trend-body');
    if (!container) return;
    if (!rows.length) {
      container.innerHTML = '<div class="empty">No architecture-trend data — repo too small, or the historical scan was skipped.</div>';
      return;
    }
    const dates = rows.map(function (r) { return r.date; });
    const propagation = rows.map(function (r) {
      return Number(((r.propagation_cost || 0) * 100).toFixed(2));
    });
    const cycles = rows.map(function (r) { return r.cycle_count || 0; });
    const infoColor = token('--color-info') || '#2563eb';
    const errColor = token('--color-error') || '#dc2626';
    const dim = getCssVar('--fg-dim');

    setChartAriaLabel(container,
      'Architecture decay trend over ' + rows.length +
      ' sampled revisions: propagation cost (percent) and dependency-cycle count.');

    const chart = mountEcharts(container);
    chart.setOption({
      tooltip: { trigger: 'axis' },
      legend: {
        data: ['Propagation cost %', 'Dependency cycles'],
        textStyle: { color: dim },
        bottom: 0,
      },
      grid: { left: 24, right: 8, top: 24, bottom: 48, containLabel: true },
      xAxis: {
        type: 'category',
        data: dates,
        axisLabel: { color: dim, rotate: 30, fontSize: 10 },
      },
      yAxis: [
        {
          type: 'value',
          name: 'Propagation %',
          position: 'left',
          nameGap: 10,
          axisLabel: { color: dim },
          nameTextStyle: { color: dim, fontSize: 10, align: 'left' },
          splitLine: { lineStyle: { color: getCssVar('--border') } },
        },
        {
          type: 'value',
          name: 'Cycles',
          position: 'right',
          minInterval: 1,
          nameGap: 10,
          axisLabel: { color: dim },
          nameTextStyle: { color: dim, fontSize: 10, align: 'left' },
          splitLine: { show: false },
        },
      ],
      series: [
        {
          name: 'Propagation cost %',
          type: 'line',
          smooth: true,
          symbol: 'circle',
          symbolSize: 6,
          yAxisIndex: 0,
          data: propagation,
          lineStyle: { color: infoColor, width: 2 },
          itemStyle: { color: infoColor },
          areaStyle: { color: infoColor, opacity: 0.08 },
        },
        {
          name: 'Dependency cycles',
          type: 'line',
          step: 'end',
          symbol: 'circle',
          symbolSize: 6,
          yAxisIndex: 1,
          data: cycles,
          lineStyle: { color: errColor, width: 2, type: 'dashed' },
          itemStyle: { color: errColor },
        },
      ],
    });
  }

  // ─── §11j Widget: Repo health timeline ─────────────────────────────
  // Overlaid 3-line chart (Combined bold; Architectural + Code lighter)
  // on a 0–100 axis with faint red/yellow/green band background, plus a
  // vanilla toggle that re-renders as three stacked small-multiples.
  function healthTrendBands(errColor, warnColor, okColor) {
    // Red / yellow / green background zones. Thresholds come from the run's
    // options snapshot (data.options.health_green_min / health_yellow_min) so
    // the chart background always matches the Rust band constants in `bands.rs`.
    // Fallback values (70 / 40) match HEALTH_GREEN_MIN / HEALTH_YELLOW_MIN for
    // data payloads that pre-date this field.
    var opts = data.options || {};
    var gMin = (opts.health_green_min != null) ? opts.health_green_min : 70;
    var yMin = (opts.health_yellow_min != null) ? opts.health_yellow_min : 40;
    var zoneColors = [errColor, warnColor, okColor];
    return {
      silent: true,
      data: [
        [{ yAxis: 0 }, { yAxis: yMin }],
        [{ yAxis: yMin }, { yAxis: gMin }],
        [{ yAxis: gMin }, { yAxis: 100 }],
      ],
      itemStyle: {
        color: function (params) {
          return zoneColors[params.dataIndex] || zoneColors[0];
        },
        opacity: 0.06,
      },
    };
  }

  function renderHealthTrend(rows, mode) {
    const container = document.getElementById('widget-health-trend-body');
    if (!container) return;
    if (rows.length < 2) {
      container.innerHTML =
        '<div class="empty">Not enough history for a health timeline — need at least 2 commits.</div>';
      return;
    }
    const view = mode || 'overlay';
    const dates = rows.map(function (r) { return r.date; });
    const arch = rows.map(function (r) { return Number((r.arch_health || 0).toFixed(2)); });
    const code = rows.map(function (r) { return Number((r.code_health || 0).toFixed(2)); });
    const combined = rows.map(function (r) { return Number((r.combined_health || 0).toFixed(2)); });

    const okColor = token('--color-success') || '#16a34a';
    const warnColor = token('--color-warning') || '#ca8a04';
    const errColor = token('--color-error') || '#dc2626';
    const fgColor = getCssVar('--fg') || '#e6edf3';
    const dim = getCssVar('--fg-dim');
    const bands = healthTrendBands(errColor, warnColor, okColor);

    // Toggle button + chart host.
    container.innerHTML =
      '<div class="widget-toolbar"><button id="ht-toggle" class="wt-btn">' +
      (view === 'overlay' ? 'Split view' : 'Overlay view') +
      '</button></div><div id="ht-charts"></div>';
    const toggle = document.getElementById('ht-toggle');
    if (toggle) {
      toggle.onclick = function () {
        renderHealthTrend(rows, view === 'overlay' ? 'split' : 'overlay');
      };
    }
    const host = document.getElementById('ht-charts');

    const baseAxis = {
      tooltip: { trigger: 'axis' },
      grid: { left: 8, right: 8, top: 28, bottom: 28, containLabel: true },
      xAxis: {
        type: 'category',
        data: dates,
        boundaryGap: false,
        axisLabel: { color: dim, rotate: 30, fontSize: 10 },
      },
      yAxis: {
        type: 'value',
        min: 0,
        max: 100,
        axisLabel: { color: dim },
        splitLine: { lineStyle: { color: getCssVar('--border') } },
      },
    };

    if (view === 'overlay') {
      setChartAriaLabel(container,
        'Repo health timeline over ' + rows.length +
        ' sampled revisions: combined, architectural, and code health (0–100).');
      const chart = mountEcharts(host);
      chart.setOption(Object.assign({}, baseAxis, {
        legend: {
          data: ['Combined', 'Architectural', 'Code'],
          textStyle: { color: dim },
          bottom: 0,
        },
        series: [
          {
            name: 'Architectural',
            type: 'line',
            smooth: true,
            symbol: 'circle',
            symbolSize: 5,
            data: arch,
            lineStyle: { color: okColor, width: 1.5, opacity: 0.7 },
            itemStyle: { color: okColor },
          },
          {
            name: 'Code',
            type: 'line',
            smooth: true,
            symbol: 'circle',
            symbolSize: 5,
            data: code,
            lineStyle: { color: warnColor, width: 1.5, opacity: 0.7 },
            itemStyle: { color: warnColor },
          },
          {
            name: 'Combined',
            type: 'line',
            smooth: true,
            symbol: 'circle',
            symbolSize: 6,
            data: combined,
            lineStyle: { color: fgColor, width: 3 },
            itemStyle: { color: fgColor },
            markArea: bands,
          },
        ],
      }));
      return;
    }

    // Split: three stacked small-multiples, same data, no recompute.
    const panels = [
      { label: 'Combined',       series: combined, color: fgColor },
      { label: 'Architectural',  series: arch,     color: okColor },
      { label: 'Code',           series: code,     color: warnColor },
    ];
    host.innerHTML = panels
      .map(function (p, i) { return '<div id="ht-sm-' + i + '" class="ht-sm"></div>'; })
      .join('');
    panels.forEach(function (p, i) {
      const el = document.getElementById('ht-sm-' + i);
      if (!el) return;
      const isLast = i === panels.length - 1;
      const c = mountEcharts(el);
      c.setOption(Object.assign({}, baseAxis, {
        title: {
          text: p.label,
          left: 8,
          top: 4,
          textStyle: { fontSize: 12, color: getCssVar('--fg') },
        },
        grid: {
          left: 8,
          right: 8,
          top: 36,
          bottom: isLast ? 24 : 8,
          containLabel: true,
        },
        xAxis: {
          type: 'category',
          data: dates,
          boundaryGap: false,
          axisLabel: {
            show: isLast,
            color: dim,
            rotate: 30,
            fontSize: 10,
          },
        },
        yAxis: {
          type: 'value',
          min: 0,
          max: 100,
          interval: 50,
          axisLabel: { color: dim, fontSize: 10 },
          splitLine: { lineStyle: { color: getCssVar('--border') } },
        },
        series: [
          {
            name: p.label,
            type: 'line',
            smooth: true,
            symbol: 'circle',
            symbolSize: 5,
            data: p.series,
            lineStyle: { color: p.color, width: 2 },
            itemStyle: { color: p.color },
            markArea: bands,
          },
        ],
      }));
    });
  }

  // Classify above-diagonal structural edges against aggregated
  // co-change data for the DSM Fusion cell-mode. Pure and DOM-free —
  // `renderArchMatrix` supplies module-level maps already rolled up
  // with the shared `modulePath`/`aggregateImportsAt`/
  // `aggregateCouplingAt` helpers and owns everything about screen
  // position (row/col, above/below the diagonal).
  //
  //   structEdges — `{ 'srcModule\x00tgtModule': importCount }`, the
  //                 exact shape `aggregateImportsAt` returns.
  //   couplingAgg — `{ 'modA\x00modB': maxDegreePct }` (modA < modB
  //                 alphabetically), from `aggregateCouplingAt`.
  //
  // Returns `{ edgeClasses, extra }`:
  //   - `edgeClasses[key]` classifies an EXISTING structural-edge key
  //     as `{ cls: 'agree', degree }` (also co-changes) or
  //     `{ cls: 'struct-only' }` (imports, never co-changes). Callers
  //     only consult this for forward (above-diagonal) edges —
  //     back-edges stay red unconditionally, in both modes.
  //   - `extra` lists `{ a, b, degree }` triples for module pairs that
  //     co-change with NO structural edge in either direction — cells
  //     Fusion mode adds that structure mode never draws.
  function classifyCells(structEdges, couplingAgg) {
    const edgeClasses = {};
    Object.keys(structEdges).forEach(function (k) {
      const parts = k.split('\x00');
      const canon = (parts[0] < parts[1]) ? k : (parts[1] + '\x00' + parts[0]);
      const deg = couplingAgg[canon];
      edgeClasses[k] = (deg === undefined)
        ? { cls: 'struct-only' }
        : { cls: 'agree', degree: deg };
    });
    const extra = [];
    Object.keys(couplingAgg).forEach(function (k) {
      const parts = k.split('\x00'); // canonical: parts[0] < parts[1]
      const fwd = k;
      const bwd = parts[1] + '\x00' + parts[0];
      if (structEdges[fwd] !== undefined || structEdges[bwd] !== undefined) return;
      extra.push({ a: parts[0], b: parts[1], degree: couplingAgg[k] });
    });
    return { edgeClasses: edgeClasses, extra: extra };
  }

  // Cell-mode legend: text labels so the encoding is never color-only
  // (WCAG 1.4.1). Shared between the fusion legend row and nowhere
  // else — kept as one small function so the four labels can't drift
  // from the tooltip's class names.
  function archMatrixLegendHtml(fwdColor, violColor, backColor) {
    // Inline styles only — this markup is injected straight into the DOM
    // (not scanned by the offline Tailwind build), so Tailwind utility
    // classes here would silently carry no rules.
    function item(color, opacity, label) {
      return '<span style="display:inline-flex;align-items:center;gap:4px;margin-right:12px;">' +
        '<i style="display:inline-block;width:10px;height:10px;border-radius:2px;' +
        'background:' + color + ';opacity:' + opacity + '"></i>' + label + '</span>';
    }
    return item(fwdColor, 0.85, 'agree — import + co-change') +
      item(fwdColor, 0.35, 'structural only') +
      item(violColor, 0.85, 'co-change only (modularity violation)') +
      item(backColor, 0.95, 'back-edge (cycle)');
  }

  function renderArchMatrix(imports, roles, coupling) {
    roles = roles || [];
    coupling = coupling || [];
    const outer = document.getElementById('widget-arch-matrix-body');
    if (!outer) return;
    if (!imports.length) {
      outer.innerHTML = '<div class="empty">No resolved import edges to matrix yet (Rust + Python + JS/TS).</div>';
      return;
    }
    // Same module roll-up + edge aggregation as the force graph.
    const archLayout = (window.Alpine && window.Alpine.store)
      ? window.Alpine.store('layout') : null;
    const userDepth = archLayout ? archLayout.archGraphDepth : 'auto';
    var edges = {};
    var nodes = {};
    var chosenDepth = (typeof userDepth === 'number') ? userDepth : 6;
    if (typeof userDepth === 'number') {
      const r = aggregateImportsAt(imports, userDepth);
      edges = r.edges;
      nodes = r.nodes;
    } else {
      for (var d = 2; d <= 6; d++) {
        const r = aggregateImportsAt(imports, d);
        edges = r.edges;
        nodes = r.nodes;
        chosenDepth = d;
        if (Object.keys(nodes).length >= 8) break;
      }
    }
    const mods = Object.keys(nodes);
    if (!mods.length) {
      outer.innerHTML = '<div class="empty">All resolved imports stay intra-module — no inter-module matrix.</div>';
      return;
    }
    // Module layer = shallowest member file's topological level (from
    // architecture-roles). Modules with no level sink to the bottom.
    const moduleLevel = {};
    for (var ri = 0; ri < roles.length; ri++) {
      const m = modulePath(roles[ri].path, chosenDepth);
      if (!m || !nodes[m]) continue;
      const lv = (typeof roles[ri].level === 'number') ? roles[ri].level : 1e9;
      if (moduleLevel[m] === undefined || lv < moduleLevel[m]) moduleLevel[m] = lv;
    }
    // Order entry points first, foundations last → forward deps read
    // above the diagonal, back-edges (cycles) below it.
    const order = mods.slice().sort(function (a, b) {
      const la = (moduleLevel[a] === undefined) ? 1e9 : moduleLevel[a];
      const lb = (moduleLevel[b] === undefined) ? 1e9 : moduleLevel[b];
      return la - lb || (a < b ? -1 : (a > b ? 1 : 0));
    });
    const idxOf = {};
    order.forEach(function (m, i) { idxOf[m] = i; });
    const n = order.length;
    const labels = order.map(function (m) {
      return m.length > 24 ? '…' + m.slice(-23) : m;
    });
    const fwdColor = token('--color-info') || '#2563eb';
    const backColor = token('--color-error') || '#dc2626';
    const violColor = token('--color-warning') || '#d97706';
    var maxCount = 1;
    Object.keys(edges).forEach(function (k) { if (edges[k] > maxCount) maxCount = edges[k]; });

    // ─── Cell-mode: 'structure' (today's rendering, default) or
    // 'fusion' (structure×history agreement classes), persisted next
    // to `archGraphLayout`. The toggle button is injected here (`wt-
    // btn`), mirroring the health-trend toggle's rebuild-on-click
    // pattern (`renderHealthTrend`'s `ht-toggle`): the button names
    // the mode a click switches INTO, and clicking re-invokes this
    // function with the flipped store value.
    const mode = (archLayout && archLayout.archMatrixMode === 'fusion') ? 'fusion' : 'structure';
    const couplingAgg = aggregateCouplingAt(coupling, chosenDepth);
    const hasCoupling = Object.keys(couplingAgg).length > 0;
    // Honest absence: Fusion mode with no coupling data degrades to
    // exactly the structure-mode rendering, plus a hint explaining why.
    const effectiveMode = (mode === 'fusion' && !hasCoupling) ? 'structure' : mode;

    outer.innerHTML =
      '<div class="widget-toolbar"><button id="wam-mode-toggle" class="wt-btn">' +
      (mode === 'fusion' ? 'Structure' : 'Fusion') + '</button></div>' +
      (mode === 'fusion' && !hasCoupling
        ? '<div style="font-size:11px;color:' + getCssVar('--fg-dim') + ';margin-bottom:6px;">No co-change data — showing structure only</div>'
        : '') +
      '<div id="wam-legend" style="font-size:11px;color:' + getCssVar('--fg-dim') + ';margin-bottom:6px;' +
      (effectiveMode === 'fusion' ? '' : 'display:none;') + '"></div>' +
      '<div id="wam-chart-host"></div>';
    const toggleBtn = document.getElementById('wam-mode-toggle');
    if (toggleBtn) {
      toggleBtn.onclick = function () {
        if (archLayout) archLayout.archMatrixMode = (mode === 'fusion') ? 'structure' : 'fusion';
        renderArchMatrix(imports, roles, coupling);
      };
    }
    const legendHost = document.getElementById('wam-legend');
    if (legendHost && effectiveMode === 'fusion') {
      legendHost.innerHTML = archMatrixLegendHtml(fwdColor, violColor, backColor);
    }
    const container = document.getElementById('wam-chart-host');
    if (!container) return;

    // cellMeta[col + '\x00' + row] parallels `cells` for the tooltip —
    // keyed by the same [col, row] pair `value` uses, carrying the
    // class + raw numbers the formatter needs without re-deriving them.
    const cellMeta = {};
    const cells = [];
    var backEdges = 0;

    if (effectiveMode !== 'fusion') {
      // Today's rendering — untouched.
      Object.keys(edges).forEach(function (k) {
        const parts = k.split('\x00');
        const r = idxOf[parts[0]]; // importer → row
        const c = idxOf[parts[1]]; // imported → col
        if (r === undefined || c === undefined) return;
        const count = edges[k];
        const isBack = r > c; // imports own-layer-or-shallower = cycle/back-edge
        if (isBack) backEdges += 1;
        cells.push({
          value: [c, r, count],
          itemStyle: {
            color: isBack ? backColor : fwdColor,
            // Forward edges fade by weight; back-edges stay loud (they're the
            // thing to notice). Floor kept high enough to read on the dark grid.
            opacity: isBack ? 0.95 : (0.5 + 0.45 * (count / maxCount)),
          },
        });
      });
    } else {
      // Fusion: reclassify above-diagonal structural cells against
      // aggregated co-change data, and add coupling-only cells
      // (`temporal-only`) the structure-mode loop never draws.
      // Below-diagonal back-edges are untouched in both modes.
      var maxCouplingDegree = 1;
      Object.keys(couplingAgg).forEach(function (k) {
        if (couplingAgg[k] > maxCouplingDegree) maxCouplingDegree = couplingAgg[k];
      });
      const classified = classifyCells(edges, couplingAgg);
      Object.keys(edges).forEach(function (k) {
        const parts = k.split('\x00');
        const r = idxOf[parts[0]];
        const c = idxOf[parts[1]];
        if (r === undefined || c === undefined) return;
        const count = edges[k];
        const isBack = r > c;
        if (isBack) {
          backEdges += 1;
          cells.push({ value: [c, r, count], itemStyle: { color: backColor, opacity: 0.95 } });
          cellMeta[c + '\x00' + r] = { cls: 'back-edge', count: count };
          return;
        }
        const info = classified.edgeClasses[k] || { cls: 'struct-only' };
        const opacity = (info.cls === 'agree')
          ? (0.45 + 0.5 * ((info.degree || 0) / maxCouplingDegree))
          : 0.35;
        cells.push({ value: [c, r, count], itemStyle: { color: fwdColor, opacity: opacity } });
        cellMeta[c + '\x00' + r] = { cls: info.cls, count: count, degree: info.degree };
      });
      classified.extra.forEach(function (ex) {
        const ia = idxOf[ex.a];
        const ib = idxOf[ex.b];
        if (ia === undefined || ib === undefined) return;
        const r = Math.min(ia, ib);
        const c = Math.max(ia, ib);
        // -2 marks a coupling-only cell: no import edge backs it, so it
        // carries no import count (distinct from the -1 diagonal guide).
        cells.push({ value: [c, r, -2], itemStyle: { color: violColor, opacity: 0.85 } });
        cellMeta[c + '\x00' + r] = { cls: 'temporal-only', count: 0, degree: ex.degree };
      });
    }
    // Diagonal guide cells. No file imports itself, so every r==c cell is
    // empty — without a marker the eye has nothing to anchor the triangle to
    // (the caption's "triangular, all-blue = clean" is otherwise invisible).
    // value[2] = -1 lets the tooltip tell a guide cell from a real edge.
    for (var di = 0; di < n; di++) {
      cells.push({
        value: [di, di, -1],
        itemStyle: { color: getCssVar('--fg-dim') || '#888', opacity: 0.22 },
      });
    }
    setChartAriaLabel(container,
      'Dependency structure matrix, ' + n + ' modules ordered by architectural layer, ' +
      Object.keys(edges).length + ' dependency cells, ' + backEdges +
      ' below-diagonal back-edges (dependency cycles / layering violations) in red' +
      (effectiveMode === 'fusion' ? '. Fusion mode: cells classified by structure×history agreement.' : '.'));

    // Square cells so the diagonal is a true 45° line rather than the shallow
    // slope a full-width stretch produces. Reserve fixed margins for the
    // (rotated) labels, size the plot box to n×n equal cells, and centre it
    // when the panel is wider than the matrix.
    const cell = Math.max(11, Math.min(26, Math.round(620 / Math.max(n, 1))));
    const padTop = 128;   // rotated column labels
    const padLeft = 156;  // row labels
    const span = n * cell;
    const cw = container.clientWidth || 900;
    const gridLeft = Math.max(padLeft, Math.round((cw - span) / 2));
    container.style.height = (padTop + span + 10) + 'px';
    // The chart mounts on the nested `#wam-chart-host`; size the widget
    // BODY (`outer`) to its content so the card grows with the matrix at
    // any module count instead of staying pinned to the template's fallback
    // height and letting a tall matrix bleed out of its card. Fullscreen's
    // `!important` body height still wins over this inline value, so
    // entering fullscreen re-fills to the viewport as before.
    outer.style.height = 'auto';

    const chart = mountEcharts(container);
    chart.setOption({
      tooltip: {
        position: 'top',
        formatter: function (p) {
          // The repeated escapeHtml(order[...]) below reads like something to
          // hoist into a local. It isn't: the guard that keeps this file
          // escaped only inspects statements that also build markup, so an
          // escape moved to its own line leaves the guard's window and takes
          // these sinks out of coverage — the exact sinks it was written for.
          const c = p.value[0];
          const r = p.value[1];
          const v = p.value[2];
          if (r === c) return escapeHtml(order[r]) + '<br/><span style="opacity:.7">diagonal (self)</span>';
          if (effectiveMode !== 'fusion') {
            return escapeHtml(order[r]) + ' &rarr; ' + escapeHtml(order[c]) +
              '<br/>' + v + ' import' + (v === 1 ? '' : 's') +
              (r > c ? '<br/><strong>back-edge — dependency cycle / layering violation</strong>' : '');
          }
          const meta = cellMeta[c + '\x00' + r] || { cls: 'struct-only', count: v };
          const label = {
            'agree': 'agree — import + co-change',
            'struct-only': 'structural only',
            'temporal-only': 'co-change only — modularity violation',
            'back-edge': 'back-edge — dependency cycle / layering violation',
          }[meta.cls] || 'structural only';
          const importsTxt = 'imports: ' + (meta.count || 0);
          const coTxt = (typeof meta.degree === 'number')
            ? ('co-change degree: ' + meta.degree.toFixed(1) + '%')
            : 'co-change degree: n/a';
          return escapeHtml(order[r]) + ' &rarr; ' + escapeHtml(order[c]) +
            ' — ' + label + '<br/>' + importsTxt + ', ' + coTxt;
        },
      },
      grid: { left: gridLeft, top: padTop, width: span, height: span, containLabel: false },
      xAxis: {
        type: 'category', data: labels, position: 'top',
        axisTick: { show: false },
        axisLabel: { rotate: 55, fontSize: 9, color: getCssVar('--fg-dim'), margin: 8 },
        // Faint column banding ONLY — banding both axes cross-hatches into the
        // heavy checkerboard that drowns the data cells.
        splitArea: { show: true, areaStyle: { color: ['transparent', 'rgba(128,128,128,0.05)'] } },
      },
      yAxis: {
        type: 'category', data: labels, inverse: true,
        axisTick: { show: false },
        axisLabel: { fontSize: 9, color: getCssVar('--fg-dim'), margin: 8 },
        splitArea: { show: false },
      },
      series: [{
        type: 'heatmap',
        data: cells,
        label: { show: false },
        itemStyle: { borderColor: getCssVar('--bg'), borderWidth: 0.5 },
        emphasis: { itemStyle: { borderColor: getCssVar('--fg'), borderWidth: 1 } },
      }],
    });

    // Cross-widget selection: emphasise the selected file's row and column
    // in the DSM. The matrix axes are module-level prefixes (at `chosenDepth`
    // segments); the bus delivers full file paths, so we truncate via the same
    // `modulePath` helper before looking up in `idxOf`. The data array is
    // sparse, so we scan `cells` rather than assuming a dense row-major layout.
    // `value[0]` is the column (imported module), `value[1]` is the row
    // (importer module). A null selection, or a path outside the visible
    // modules, downplays everything back to neutral.
    window._codeloreRegisterSelectionListener('dsm', function (selectedPath) {
      chart.dispatchAction({ type: 'downplay' });
      if (!selectedPath) return;
      const mod = modulePath(selectedPath, chosenDepth);
      const idx = idxOf[mod];
      if (idx === undefined) return;
      const indices = [];
      for (var k = 0; k < cells.length; k++) {
        if (cells[k].value[0] === idx || cells[k].value[1] === idx) indices.push(k);
      }
      chart.dispatchAction({ type: 'highlight', seriesIndex: 0, dataIndex: indices });
    });
  }


