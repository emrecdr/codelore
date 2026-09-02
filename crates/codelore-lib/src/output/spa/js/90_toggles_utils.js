  // ─── §14 Controls: hotspot color-mode toggles ────────────────────
  //
  // Theme toggle itself is owned by Alpine (`$store.theme.isDark`
  // registered in template.html). The `Alpine.effect` there flips
  // `<html data-theme>` AND fires every callback in
  // `window._codeloreRerenderers`, so this file just appends to that
  // registry — no theme-toggle init function lives here.

  function initHotspotColorToggles() {
    // Gap #4 migration: button row → DaisyUI `tabs tabs-boxed`. The
    // selector matches `[role="tab"]` so the same handler works on
    // both the old class-`toggle` markup (if any cached SPA HTML is
    // still in the wild during the cut-over) and the new tab markup.
    const bar = document.getElementById('hotspot-color-toggles');
    if (!bar) return;
    const buttons = bar.querySelectorAll('button[role="tab"], button.toggle');
    for (var i = 0; i < buttons.length; i++) {
      buttons[i].addEventListener('click', function (evt) {
        const mode = evt.currentTarget.getAttribute('data-mode');
        // Update active state — `tab-active` for DaisyUI tabs and
        // `active` for any legacy markup. Both classes are written
        // as complete literals so the Tailwind v4 `@source` scanner
        // sees them (otherwise dynamic suffixes drop out of the bundle).
        // `aria-selected` mirrors the visual state for the WAI-ARIA
        // tabs pattern — without it screen readers see every tab as
        // equally focusable but none as "selected".
        for (var j = 0; j < buttons.length; j++) {
          const isCurrent = (buttons[j] === evt.currentTarget);
          buttons[j].classList.toggle('tab-active', isCurrent);
          buttons[j].classList.toggle('active', isCurrent);
          buttons[j].setAttribute('aria-selected', isCurrent ? 'true' : 'false');
          // Roving tabindex must follow a MOUSE selection too: the WAI-ARIA
          // tabs pattern makes the selected tab the single tab stop, and
          // without this line a click left tabindex on the previously
          // arrow-selected tab — Tab landed on a non-selected tab, and one
          // arrow press silently changed the lens (arrows activate on move).
          buttons[j].setAttribute('tabindex', isCurrent ? '0' : '-1');
        }
        // Wrap the re-render in startViewTransition so the colour-
        // mode swap smoothly crossfades. On unsupported
        // browsers the wrapper is a synchronous no-op and the
        // re-render runs identically. Updating `currentHotspotColorMode`
        // inside the callback ensures the theme-toggle re-render
        // path sees the active mode too.
        startViewTransition(function () {
          currentHotspotColorMode = mode;
          // Leaving bivariate hides the legend — drop any active quadrant
          // brush so the map isn't left dimmed with no legend to clear it.
          if (mode !== 'bivariate') {
            const bs = window.Alpine && window.Alpine.store && window.Alpine.store('brush');
            if (bs && bs.cell) bs.clear();
          }
          renderHotspotCirclePack(data.hotspots || [], mode);
        });
      });
    }
  }


  // ─── §15 Utility helpers ──────────────────────────────────────────

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
            // Every field a colour lens reads must be copied here — this
            // literal is the SOLE producer of `metrics`, and the AI lens
            // rendered the whole map as "no data" grey for months because
            // ai_pct wasn't in it while the table two panels down read the
            // raw row and showed real percentages.
            child.metrics = {
              revisions: row.revisions,
              cognitive: row.cognitive,
              cognitive_health: row.cognitive_health,
              hotspot_score: row.hotspot_score,
              ai_pct: row.ai_pct,
              mi: row.mi,
              mi_rank: row.mi_rank,
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
  // Test hook, following the window._codelore* convention (_codeloreShowDetail,
  // _codoreRerenderers): the circle-pack renders to an ECharts CANVAS, so the
  // metrics copy above is unreachable from the DOM — exposing the builder lets
  // the browser suite pin that every colour-lens field survives the copy.
  window._codeloreBuildFsHierarchy = buildFsHierarchy;

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

  // Unique author list for the offboarding picker.
  // Uses entity_ownership (one row per (path, author) tuple). Sorted
  // for stable rendering across reloads.
  function computeUniqueAuthors(entityOwnership) {
    const set = new Set();
    for (var i = 0; i < entityOwnership.length; i++) {
      const author = entityOwnership[i].author;
      if (author) set.add(author);
    }
    return Array.from(set).sort();
  }

  // Coupling arc overlay helpers.

  // Quadratic Bezier path from (x1,y1) to (x2,y2) with the control
  // point offset perpendicular to the chord. curveness ∈ [0, 1];
  // 0 = straight line, 0.25 = the CodeScene-equivalent gentle arc.
  // Returned in SVG path syntax for ECharts' `type: 'path'` shape.
  function arcPath(x1, y1, x2, y2, curveness) {
    const mx = (x1 + x2) / 2;
    const my = (y1 + y2) / 2;
    const dx = x2 - x1;
    const dy = y2 - y1;
    const cx = mx + (-dy * curveness);
    const cy = my + (dx * curveness);
    return 'M ' + x1 + ',' + y1 + ' Q ' + cx + ',' + cy + ' ' + x2 + ',' + y2;
  }

  // Build the arc descriptors for the file the user clicked. Returns
  // an array of {x1, y1, x2, y2, opacity, lineWidth} — empty array
  // means "no overlay" (renders as a silent series). Per Gap #2
  // accepted: top-5 by Fisher significance (lower p = more
  // significant = drawn first). Width encodes coupling degree so
  // strongly-coupled pairs read thicker — two extra perceptual
  // dimensions on a primitive CodeScene leaves flat.
  function buildCouplingArcs(filePath, nodePositions, couplingRows) {
    if (!filePath || !nodePositions || !couplingRows || !couplingRows.length) {
      return [];
    }
    const selfPos = nodePositions.get(filePath);
    if (!selfPos) return [];
    // Filter to rows that touch filePath; sort by p ASC; take top-5.
    const matching = [];
    for (var i = 0; i < couplingRows.length; i++) {
      const r = couplingRows[i];
      if (r.entity_a === filePath || r.entity_b === filePath) matching.push(r);
    }
    matching.sort(function (a, b) {
      const pa = (typeof a.fisher_p === 'number') ? a.fisher_p : 1;
      const pb = (typeof b.fisher_p === 'number') ? b.fisher_p : 1;
      return pa - pb;
    });
    const top = matching.slice(0, 5);
    const arcs = [];
    for (var k = 0; k < top.length; k++) {
      const row = top[k];
      const peer = (row.entity_a === filePath) ? row.entity_b : row.entity_a;
      const peerPos = nodePositions.get(peer);
      if (!peerPos) continue;
      // Opacity ← (1 - fisher_p). p=0.001 → ~1.0, p=0.05 → 0.95.
      // Floor at 0.4 so even weakly-significant arcs remain visible.
      const opacity = Math.max(0.4, 1.0 - (row.fisher_p || 0));
      // Width ← coupling degree (% co-change). Cap at 7px so a
      // 100%-coupled pair doesn't overwhelm a 20%-coupled one.
      const lineWidth = 1 + Math.min(6, (row.degree || 0) / 8);
      arcs.push({
        x1: selfPos.x, y1: selfPos.y,
        x2: peerPos.x, y2: peerPos.y,
        opacity: opacity,
        lineWidth: lineWidth,
        // Partner identity + strength, so the map can emphasise the peer
        // circle and name it (the arc itself stays silent — clicks fall
        // through to the circles — so identity rides on the leaf, not the line).
        peer: peer,
        degree: row.degree,
        shared: row.shared,
      });
    }
    return arcs;
  }

  // Partial setOption update: refresh only the arc series (series[1])
  // without touching the circle-pack series[0]. Avoids the d3.pack
  // re-layout on every click. The `{}` for series[0] tells ECharts
  // "no changes here" via index-based merge.
  function updateCouplingArcs() {
    if (!lastHotspotChart || !lastHotspotNodePositions) return;
    if (lastHotspotChart.isDisposed && lastHotspotChart.isDisposed()) return;
    const arcs = buildCouplingArcs(
      selectedCouplingFile,
      lastHotspotNodePositions,
      data.coupling || []
    );
    // Mutate the module-scoped `arcData` array in place so the arc
    // renderItem's closure-captured reference keeps pointing at live
    // data. Reassigning would orphan the closure and the click flow
    // would silently render stale arcs.
    arcData.length = 0;
    for (var ai = 0; ai < arcs.length; ai++) {
      const a = arcs[ai];
      arcData.push({ value: [a.x1, a.y1], _arc: a });
    }
    // Emphasise the selected file + its coupling partners as outlined circles
    // so it is clear WHICH files are coupled, not merely that arcs exist. The
    // partner circles keep their normal leaf tooltip (name + metrics on hover).
    const peers = new Set();
    for (var pi = 0; pi < arcs.length; pi++) peers.add(arcs[pi].peer);
    if (lastCirclePackData) {
      for (var ci = 0; ci < lastCirclePackData.length; ci++) {
        const it = lastCirclePackData[ci];
        if (!it || !it._raw || !it.metrics) continue;
        it._raw.couplingSelected =
          !!selectedCouplingFile && it.fullPath === selectedCouplingFile;
        it._raw.couplingPeer = peers.has(it.fullPath);
      }
    }
    lastHotspotChart.setOption({
      series: [
        lastCirclePackData ? { data: lastCirclePackData } : {},
        { data: arcData },
      ],
    });
  }

  // Partial setOption update: re-tint per-leaf opacity to emphasise the
  // brushed bivariate quadrant (members bright, non-members dimmed), leaving
  // the arc series[1] untouched (`{}` = no change). Mirrors updateCouplingArcs'
  // index-merge pattern. `brushedPaths === null` restores default leaf opacity.
  function updateHotspotBrush() {
    if (!lastHotspotChart || !lastCirclePackData) return;
    if (lastHotspotChart.isDisposed && lastHotspotChart.isDisposed()) return;
    for (var i = 0; i < lastCirclePackData.length; i++) {
      const item = lastCirclePackData[i];
      if (!item || !item._raw || !item.metrics) continue; // leaves only
      item._raw.opacity = !brushedPaths
        ? 0.85
        : (brushedPaths.has(item.fullPath) ? 0.95 : 0.12);
    }
    lastHotspotChart.setOption({ series: [{ data: lastCirclePackData }, {}] });
  }

  // Stable palette assignment for author colors. A discrete categorical
  // palette tuned for dark-background readability; cycles if there are
  // more authors than colors.
  function makeAuthorPalette(authors) {
    // Palette read from CSS custom properties so light theme
    // gets a deeper-saturation set that's readable on white cards.
    // The 15-token chain mirrors the previous hard-coded array;
    // `token()` is theme-aware and cache-invalidated on rerender.
    const palette = [
      token('--chart-palette-1'),  token('--chart-palette-2'),  token('--chart-palette-3'),
      token('--chart-palette-4'),  token('--chart-palette-5'),  token('--chart-palette-6'),
      token('--chart-palette-7'),  token('--chart-palette-8'),  token('--chart-palette-9'),
      token('--chart-palette-10'), token('--chart-palette-11'), token('--chart-palette-12'),
      token('--chart-palette-13'), token('--chart-palette-14'), token('--chart-palette-15'),
    ];
    const sorted = authors.slice().sort();
    const out = {};
    for (var i = 0; i < sorted.length; i++) {
      out[sorted[i]] = palette[i % palette.length];
    }
    return out;
  }
})();
