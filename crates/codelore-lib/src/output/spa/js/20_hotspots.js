  // ─── §8  Widget: hotspot circle-pack (signature CodeScene view) ──

  function renderHotspotCirclePack(rows, colorMode) {
    const container = document.getElementById('widget-hotspot-circle-pack-body');
    if (!container) return;
    if (!rows.length) {
      container.innerHTML = '<div class="empty">No hotspots to display. ' +
        'The repository may be too small, or thresholds filtered everything out.</div>';
      return;
    }
    colorMode = colorMode || 'bivariate';
    // Clear any prior ECharts instance so toggles re-render cleanly.
    container.innerHTML = '';

    // Build a primary-author map (path → author with max added LoC)
    // for the W7 knowledge-map mode. Computed once per render call.
    const primaryAuthorByPath = computePrimaryAuthorByPath(data.entity_ownership || []);
    const authorPalette = makeAuthorPalette(
      Array.from(new Set(Object.values(primaryAuthorByPath)))
    );

    // Build a path → clone-group-count map for the 'clones'
    // colour mode. `data.clones` is the per-file overlay computed
    // by `output/spa.rs::run_clone_summary`; one entry per path
    // with ≥ 1 clone family. Falls back to an empty object when
    // the payload omits the field (older fixtures, no clones
    // detected). `maxCloneGroups` anchors the heatmap.
    const cloneCountByPath = {};
    let maxCloneGroups = 0;
    const cloneRows = data.clones || [];
    for (var ci = 0; ci < cloneRows.length; ci++) {
      const cr = cloneRows[ci];
      cloneCountByPath[cr.path] = cr.groups;
      if (cr.groups > maxCloneGroups) maxCloneGroups = cr.groups;
    }
    const cloneScale = maxCloneGroups || 1;

    // Build a path → composite code-health band map for the 'bivariate'
    // colour mode. `data.code_health` is the composite-score overlay from
    // the code-health analysis; each entry carries the pre-computed `band`
    // (green / yellow / red). Falls back to an empty object when the payload
    // omits the field (older fixtures, analysis not run). Mirrors the
    // cloneCountByPath pattern above.
    const bandByPath = {};
    (data.code_health || []).forEach(function (r) { bandByPath[r.path] = r.band; });

    // Step 1: build a filesystem-style hierarchy from flat HotspotRow[].
    // Each row is { path, revisions, cognitive, cognitive_health, hotspot_score }.
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
    // d3.pack lays out into a square [0, side] × [0, side]; on a panel
    // wider (or taller) than the chosen `side`, the pack would sit in
    // the top-left of the canvas. Translate every node's (x, y) so
    // the square is centred — this offset needs to land BEFORE any
    // downstream consumer (renderItem closure, arc anchors,
    // `lastHotspotNodePositions`) reads the coords, otherwise the
    // arcs would draw at the un-offset positions while the circles
    // moved.
    const xOffset = Math.max(0, (containerWidth - side) / 2);
    const yOffset = Math.max(0, (containerHeight - side) / 2);
    if (xOffset > 0 || yOffset > 0) {
      root.each(function (n) {
        n.x += xOffset;
        n.y += yOffset;
      });
    }

    // Step 3: feed the laid-out nodes into ECharts as a custom series.
    // The custom series renders one shape per node; we draw circles
    // sized + positioned exactly per d3's layout. Color encodes
    // cognitive complexity (leaves only) on a yellow→red ramp.
    const chart = mountEcharts(container);
    // Wheel-zoom + drag-pan on the canvas. ECharts `type: 'custom'`
    // doesn't support `roam` natively, so we layer CSS-transform
    // pan/zoom on top — double-click resets. Same affordance the
    // Architecture force-graph gets from `series.roam: true`.
    attachCanvasZoom(container);
    // Register the reset handler so the top-right reset button
    // (installed at boot once all widgets have rendered) calls
    // back into the canvas-zoom reset.
    window._codeloreResetZoomHandlers['widget-hotspot-circle-pack'] = function () {
      if (container && typeof container._codeloreZoomReset === 'function') {
        container._codeloreZoomReset();
      }
    };
    const nodes = root.descendants();
    const maxCognitive = nodes.reduce(function (acc, n) {
      const cog = n.data.metrics ? n.data.metrics.cognitive : 0;
      return Math.max(acc, cog);
    }, 0) || 1;

    // P75 of the run's hotspot_score distribution. The ring overlay
    // marks any leaf at or above this as "in the top-quartile of
    // hotspot risk for this analysis." Uses the project's standard
    // percentile-rank approach (matches the MI bands); absolute
    // scores still appear in tooltips for cross-repo comparability.
    const hotspotScores = nodes
      .filter(function (n) { return n.data.metrics && n.data.metrics.hotspot_score != null; })
      .map(function (n) { return n.data.metrics.hotspot_score; })
      .sort(function (a, b) { return a - b; });
    const hotspotP75 = hotspotScores.length
      ? hotspotScores[Math.floor(hotspotScores.length * 0.75)]
      : Infinity;

    // Stash the laid-out node positions in module scope so
    // updateCouplingArcs() can do partial setOption updates on click
    // without re-running buildFsHierarchy + d3.pack.
    lastHotspotChart = chart;
    lastHotspotNodePositions = new Map();
    for (var ni = 0; ni < nodes.length; ni++) {
      const n = nodes[ni];
      if (n.data && n.data.fullPath) {
        lastHotspotNodePositions.set(n.data.fullPath, { x: n.x, y: n.y, r: n.r });
      }
    }

    // Declare the data arrays as `let` here so the inline `.map(...)`
    // expressions below (inside setOption) can assign back into them.
    // The renderItem callbacks close over these names and read each
    // item's render payload via `[params.dataIndex]._raw` / `._arc`.
    //
    // ECharts 6 dropped the older path for passing structured per-item
    // data into custom-series renderItem: string-keyed `api.value()`
    // returns NaN, and numeric `api.value(N)` coerces object values
    // through Number(...) so they also come back as NaN. The
    // closure-from-data-array pattern is the documented, stable
    // ECharts 6 way to pass per-item structured data through to the
    // renderItem callback. Sibling fields (name, fullPath, metrics,
    // depth, leafCount) stay on the data item so the tooltip formatter
    // — which DOES receive the full data item via `params.data` in
    // ECharts 6 — keeps working untouched.
    let circlePackData = [];
    // `arcData` lives at module scope (declared near the top of the
    // IIFE) so updateCouplingArcs() can mutate the same reference the
    // arc renderItem closes over. Reset its length here so the per-
    // render reset doesn't reassign and break the closure.
    arcData.length = 0;

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
          // When hovering the selected file, list its coupling partners inline
          // (basename + co-change %), so the coupled set is readable in one
          // place — not just inferable from the arcs on the map.
          let couplingLine = '';
          if (selectedCouplingFile && d.fullPath === selectedCouplingFile && arcData.length) {
            const partners = arcData
              .map(function (it) {
                const a = it._arc || {};
                const nm = (a.peer || '').split('/').pop();
                return escapeHtml(nm) + ' (' + Math.round(a.degree || 0) + '%)';
              })
              .join(', ');
            couplingLine =
              '<br/><span style="opacity:.75">coupled with: ' + partners + '</span>';
          }
          return '<b>' + escapeHtml(d.fullPath) + '</b>' +
            '<br/>revisions: ' + m.revisions +
            '<br/>cognitive: ' + m.cognitive.toFixed(0) +
            '<br/>cognitive health: ' + m.cognitive_health.toFixed(1) +
            '<br/>hotspot score: ' + m.hotspot_score.toFixed(2) +
            couplingLine;
        },
      },
      series: [{
        type: 'custom',
        coordinateSystem: 'none',
        // ECharts 6 dropped string-keyed `api.value()` lookups for
        // custom-series renderItem callbacks; only numeric dimension
        // indices into the data item's `value` array resolve. We carry
        // the per-leaf render payload at `value[2]` and read it via
        // `api.value(2)`. Sibling properties on the data item (name,
        // fullPath, metrics, depth, leafCount) remain readable via
        // `params.data` in the tooltip formatter, which is a separate
        // ECharts callback context where `params.data` is preserved.
        renderItem: function (params, api) {
          // Read the render payload via closure over `circlePackData`.
          // ECharts 6 coerces non-numeric values from api.value(N) to
          // NaN, so we cannot pack the payload into `value[N]`. The
          // closure-from-data-array pattern is the documented escape.
          const item = circlePackData[params.dataIndex];
          const datum = item ? item._raw : null;
          if (!datum) return null;
          // Directories (non-leaf nodes) carry `metrics: null` on the
          // data item. They render as the giant transparent containers
          // around the actual files, so they must NOT capture pointer
          // events — otherwise ECharts' first-match tooltip hit-test
          // always picks the outermost root node and the tooltip shows
          // "root" for every hover. `silent: true` lets pointer events
          // pass through to the leaf circles painted on top.
          const isDirectory = !item.metrics;
          // When a file is selected, outline it and its coupling partners in
          // info-blue so the coupled set is legible on the map (the selected
          // file a touch heavier than its partners). Otherwise the leaf keeps
          // its normal stroke.
          const coupled = datum.couplingSelected || datum.couplingPeer;
          const innerCircle = {
            type: 'circle',
            shape: {
              cx: datum.x,
              cy: datum.y,
              r: datum.r,
            },
            style: api.style({
              fill: datum.color,
              stroke: coupled ? token('--color-info') : datum.stroke,
              lineWidth: datum.couplingSelected ? 3 : (datum.couplingPeer ? 2 : 1),
              opacity: datum.opacity,
            }),
            silent: isDirectory,
          };
          // Top-quartile leaves get a yellow ring overlay. Drawn
          // first (lower in z-order) so the inner circle paints on
          // top — preserves the existing color encoding. Ring stroke
          // uses the cached `token()` so theme toggles see the new
          // --color-warning via registerThemeRerender's cache flush.
          if (datum.isHotspot) {
            return {
              type: 'group',
              silent: isDirectory,
              children: [
                {
                  type: 'circle',
                  shape: { cx: datum.x, cy: datum.y, r: datum.r + 2.5 },
                  style: {
                    fill: 'transparent',
                    stroke: token('--color-warning'),
                    lineWidth: 2,
                    opacity: 0.85,
                  },
                  silent: isDirectory,
                },
                innerCircle,
              ],
            };
          }
          return innerCircle;
        },
        zlevel: 1,
        data: (circlePackData = nodes
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
            } else if (colorMode === 'clones') {
              // Structural-duplication overlay. `cloneCountByPath`
              // came from `data.clones` (see `output/spa.rs::run_clone_summary`).
              // Files outside any clone family render neutral grey so
              // they sit visually behind the heat colours on actual
              // clone hotspots. The heatmap colour scales by the
              // max group count across the whole dashboard so the
              // distribution is per-repo relative, not absolute.
              const groups = cloneCountByPath[n.data.fullPath] || 0;
              if (groups === 0) {
                leafColor = 'rgba(140, 140, 140, 0.55)';
              } else {
                leafColor = heatmapColor(Math.min(1, groups / cloneScale));
              }
            } else if (colorMode === 'health') {
              // Code Health Map mode — colours by the COMPOSITE code-health
              // band (`data.code_health` rows: green / yellow / red from the
              // code-health analysis), which is exactly what this lens's
              // tooltip/label claim. Sourcing the band — not the hotspots
              // `cognitive_health` proxy bounded to [60, 100] — is what makes
              // the red band reachable and keeps the lens consistent with the
              // bivariate map and its legend (same `bandByPath`). Paths absent
              // from the composite (non-Tier-1 source, analysis skipped) fall
              // back to the neutral grey the other lenses use for "no data".
              // The cognitive-health proxy stays on the surfaces that name it
              // honestly (table / drawer / tooltip metric rows).
              leafColor = bandLeafColor(bandByPath[n.data.fullPath]);
            } else if (colorMode === 'friction') {
              // Technical Debt Friction mode — continuous heat ramp
              // on hotspot_score. The formula
              // `percentile_rank(revisions) × percentile_rank(cognitive)
              // × (100 − cognitive_health) / 4` already intersects activity
              // with unhealthy code (Tornhill 2018 score, range [0,10]),
              // so this is pure SQL → ramp surfacing. OKLCH interpolation
              // via heatRamp keeps the midpoint perceptually correct.
              if (!m || m.hotspot_score == null) {
                leafColor = token('--color-base-content');
              } else {
                leafColor = heatRamp(Math.max(0, Math.min(1, m.hotspot_score / 10)));
              }
            } else if (colorMode === 'knowledge-loss') {
              // Knowledge Loss Map + Off-boarding Sim — collapsed
              // into one mode. Blue = current team owns the
              // file; red = primary author is in the offboarding
              // scenario's `departed` set; dim = no author data. The
              // user-driven `departed` list comes from the dropdown in
              // template.html — toggling it fires the Alpine.effect
              // bridge which re-runs this render via the rerenderer
              // registry, with the token cache flushed first
              // (registerThemeRerender wraps it).
              const author = primaryAuthorByPath[n.data.fullPath];
              if (!author) {
                leafColor = token('--color-base-content');
              } else {
                const scenarioStore = (window.Alpine && window.Alpine.store)
                  ? window.Alpine.store('scenario')
                  : null;
                const isDeparted = scenarioStore
                  && scenarioStore.departed.indexOf(author) >= 0;
                leafColor = isDeparted
                  ? token('--color-error')
                  : token('--color-info');
              }
            } else if (colorMode === 'bivariate') {
              // Health × activity in one glyph: band (green/yellow/red) ×
              // hotspot activity (low/med/high). The danger quadrant
              // (red × high) is the darkest/most saturated cell — visible
              // without swapping lenses. Missing band → neutral grey.
              leafColor = bivariateColor(
                bandByPath[n.data.fullPath],
                m ? m.hotspot_score : null
              );
            } else {
              leafColor = heatmapColor(ratio);
            }
            const color = isLeaf
              ? leafColor
              : 'rgba(255, 255, 255, 0.02)';
            const stroke = isLeaf
              ? 'rgba(0, 0, 0, 0.3)'
              : 'rgba(255, 255, 255, 0.15)';
            // Ring overlay: tag leaves whose hotspot_score sits
            // in the top quartile of the run. renderItem reads
            // `_raw.isHotspot` and wraps the leaf in a yellow ring.
            const isHotspot = isLeaf && m && m.hotspot_score != null
              && m.hotspot_score >= hotspotP75;
            // `value[0]`, `value[1]` carry the d3-laid-out (x, y) for
            // ECharts' coordinate system. `_raw` is the render payload
            // renderItem reads via closure over `circlePackData`
            // (see explanation above the `let circlePackData = []`).
            // Sibling fields (name, fullPath, metrics, …) drive the
            // tooltip formatter via `params.data`.
            return {
              value: [n.x, n.y],
              _raw: {
                x: n.x, y: n.y, r: n.r,
                color: color, stroke: stroke,
                opacity: isLeaf ? 0.85 : 1,
                isHotspot: isHotspot,
              },
              name: n.data.name || 'root',
              fullPath: n.data.fullPath || '',
              metrics: m || null,
              depth: n.depth,
              leafCount: n.leaves ? n.leaves().length : 0,
            };
          })),
      }, {
        // Second custom series for the coupling arc overlay.
        // Drives off the shared coordinateSystem ('none' = raw pixel
        // coords from d3.pack), so arcs anchor exactly on the circle
        // centres. zlevel: 2 paints above the circle pack. `silent:
        // true` keeps clicks falling through to the leaves below.
        // Initial data computed from the current `selectedCouplingFile`
        // (null on first render → empty array → invisible series).
        type: 'custom',
        coordinateSystem: 'none',
        zlevel: 2,
        silent: true,
        // Read the arc payload via closure over `arcData` — same
        // ECharts 6 pattern as the inner circle-pack series above.
        renderItem: function (params, api) {
          const item = arcData[params.dataIndex];
          const arc = item ? item._arc : null;
          if (!arc) return null;
          return {
            type: 'path',
            shape: { d: arcPath(arc.x1, arc.y1, arc.x2, arc.y2, 0.25) },
            style: {
              stroke: token('--color-warning'),
              fill: 'none',
              opacity: arc.opacity,
              lineWidth: arc.lineWidth,
            },
            silent: true,
          };
        },
        // `value[0]`, `value[1]` anchor the arc on its first endpoint;
        // `_arc` carries the full payload that renderItem reads via
        // closure over `arcData` (module scope, never reassigned —
        // mutated in place so the closure stays live across calls
        // from updateCouplingArcs).
        data: (function () {
          const arcs = buildCouplingArcs(
            selectedCouplingFile,
            lastHotspotNodePositions,
            data.coupling || []
          );
          for (var ai = 0; ai < arcs.length; ai++) {
            const a = arcs[ai];
            arcData.push({ value: [a.x1, a.y1], _arc: a });
          }
          return arcData;
        })(),
      }],
    });

    // Stash the built render payload in module scope so updateHotspotBrush()
    // can re-tint opacities via a partial setOption, and re-apply an active
    // brush after a full re-render (theme toggle etc.).
    lastCirclePackData = circlePackData;
    if (brushedPaths) updateHotspotBrush();

    chart.on('click', function (params) {
      const d = params && params.data;
      if (d && d.fullPath && d.metrics) {
        // Clicking a leaf surfaces its coupling partners AND opens the
        // drawer. Route through _codeloreShowDetail so the click also
        // broadcasts the selection — the 'hotspot-map' listener then sets
        // selectedCouplingFile + redraws the arcs, so we must NOT do that
        // here too (double redraw). The direct arc update stays only on the
        // no-broadcast fallback path.
        if (window._codeloreShowDetail) {
          window._codeloreShowDetail(d.fullPath);
        } else {
          selectedCouplingFile = d.fullPath;
          updateCouplingArcs();
          showFileDetailDrawer(d.fullPath, data);
        }
      }
    });

    // Clicking the canvas background (no shape under the pointer) clears the
    // selection. `e.target` is falsy for background clicks in zrender's event
    // model. The map now PUBLISHES on leaf click, so a background click must
    // clear the shared focus across every widget — not just the local arcs —
    // to stay symmetric. Broadcasting a clear fans out to the 'hotspot-map'
    // listener, which nulls selectedCouplingFile + redraws. Fallback (Alpine
    // absent): clear the arcs directly.
    chart.getZr().on('click', function (e) {
      if (!e.target) {
        const sel =
          window.Alpine && window.Alpine.store && window.Alpine.store('selection');
        if (sel) {
          sel.clear();
        } else {
          selectedCouplingFile = null;
          updateCouplingArcs();
        }
      }
    });

    // Cross-widget selection: when a file is selected in ANY widget, light up
    // its coupling arcs on the map — the same overlay a direct leaf-click
    // shows. Reuses the existing selectedCouplingFile + updateCouplingArcs
    // machinery, so the map participates in the shared focus without a
    // second highlight mechanism. A null selection clears the arcs.
    window._codeloreRegisterSelectionListener('hotspot-map', function (selectedPath) {
      selectedCouplingFile = selectedPath || null;
      updateCouplingArcs();
    });

    // Bivariate quadrant brush: emphasise the set / dim the rest by
    // recomputing per-leaf opacity. Registered here (mirrors the selection
    // listener) so it closes over the fresh render; re-fires via the brush
    // store's Alpine.effect fan-out.
    window._codeloreRegisterBrushListener('hotspot-map', function (cell, paths) {
      brushedPaths = (paths && paths.length) ? new Set(paths) : null;
      updateHotspotBrush();
    });

    renderBivariateLegend();
  }

  // 3×3 bivariate legend: a small grid keyed to BIVARIATE_PALETTE, axes
  // labeled health (green→red, top→bottom) × activity (low→high, left→right).
  // Populates the legend mount whenever the circle-pack renders; a no-op if the
  // mount is absent. Only visible while the bivariate mode is active — in the
  // other colour modes the legend would describe an encoding not on screen, so
  // it hides itself. The palette is a fixed CVD-tuned hex set (not DaisyUI theme
  // tokens) on purpose: the health×activity blend must stay deterministic and
  // lightness-monotonic regardless of theme.
  function renderBivariateLegend() {
    const mount = document.getElementById('bivariate-legend');
    if (!mount) return;
    mount.style.display = (currentHotspotColorMode === 'bivariate') ? '' : 'none';
    const cells = BIVARIATE_PALETTE.map(function (c, i) {
      const hb = Math.floor(i / 3);
      const ab = i % 3;
      return '<div data-biv-cell data-hb="' + hb + '" data-ab="' + ab + '" '
        + 'style="width:14px;height:14px;background:' + c + ';cursor:pointer;outline-offset:1px" '
        + 'title="health ' + (['healthy', 'warning', 'unhealthy'][hb])
        + ' × activity ' + (['low', 'med', 'high'][ab])
        + ' — click to brush this quadrant"></div>';
    }).join('');
    mount.innerHTML =
      '<div class="text-xs opacity-70 mb-1">Health × Activity</div>' +
      '<div style="display:grid;grid-template-columns:repeat(3,14px);gap:2px">' + cells + '</div>' +
      '<div class="text-xs opacity-50 mt-1">↓ less healthy&nbsp;&nbsp;→ more active</div>';

    // Legend cell → quadrant set-brush. Band from data.code_health (same
    // source as the circle-pack's bandByPath); activity from data.hotspots.
    // Clicking the active cell again clears.
    const bandByPath = {};
    (data.code_health || []).forEach(function (r) { bandByPath[r.path] = r.band; });
    const cellEls = mount.querySelectorAll('[data-biv-cell]');
    for (var i = 0; i < cellEls.length; i++) {
      cellEls[i].addEventListener('click', function (evt) {
        const store = window.Alpine && window.Alpine.store && window.Alpine.store('brush');
        if (!store) return;
        const hb = Number(evt.currentTarget.getAttribute('data-hb'));
        const ab = Number(evt.currentTarget.getAttribute('data-ab'));
        if (store.isActive(hb, ab)) { store.clear(); return; }
        const paths = (data.hotspots || []).filter(function (h) {
          return healthBucket(bandByPath[h.path]) === hb
            && activityBucket(h.hotspot_score) === ab;
        }).map(function (h) { return h.path; });
        store.set([hb, ab], paths);
      });
      wireRowKbActivation(cellEls[i]); // role=button + tabindex + Enter/Space → click
    }

    // Legend is itself a brush subscriber: outline the active quadrant cell.
    if (window._codeloreRegisterBrushListener) {
      window._codeloreRegisterBrushListener('bivariate-legend', function (cell) {
        const m = document.getElementById('bivariate-legend');
        if (!m) return;
        const cs = m.querySelectorAll('[data-biv-cell]');
        for (var k = 0; k < cs.length; k++) {
          const on = !!cell
            && Number(cs[k].getAttribute('data-hb')) === cell[0]
            && Number(cs[k].getAttribute('data-ab')) === cell[1];
          cs[k].style.outline = on ? '2px solid var(--color-base-content)' : '';
        }
      });
    }
  }


  // ─── §9  Widget: hotspot table (sortable drill-down of §8) ────────

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
      { key: 'cognitive_health', label: 'Cognitive Health', cls: 'num', kind: 'number', defaultDir: 1, defKey: 'cognitive_health' },
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
      // DaisyUI `table table-zebra` provides striped rows + consistent
      // typography on top of the inline `.table-container table { ... }`
      // rules. The two co-exist: inline rules win on background-color
      // (var(--bg-elev-2)) for stylistic continuity; DaisyUI's font
      // tokens layer on top.
      let html = '<table class="table table-zebra"><thead><tr>';
      for (var i = 0; i < COLUMNS.length; i++) {
        const c = COLUMNS[i];
        const active = (c.key === sortKey);
        const indicator = active
          ? (sortDir > 0 ? '▲' : '▼')
          : '';
        const tip = c.defKey ? buildTooltipHtml(c.defKey) : '';
        html += '<th scope="col" class="' + (active ? 'active' : '') + '"' +
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
          // A click (or keyboard activation) on the metric-help "?" button,
          // which lives inside the <th>, must not also sort the column.
          if (evt.target.closest && evt.target.closest('.tooltip-trigger')) {
            return;
          }
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

    async function renderNextPage(count) {
      const tbody = container.querySelector('#hotspot-tbody');
      if (!tbody) return;
      // Filter matched nothing: show an inline message instead of a blank
      // body (which reads as "the table broke"). The whole-dataset-empty
      // case is handled by the earlier `No hotspot rows.` return, so an
      // empty view here always means an active filter with no matches.
      if (filteredView.length === 0) {
        tbody.innerHTML = '<tr><td class="empty" colspan="99">No paths match “' +
          escapeHtml(filterText) + '”.</td></tr>';
        renderedRows = 0;
        refreshActions();
        return;
      }
      // Chunk the rebuild — `Show all` historically called this with
      // `Infinity` and blocked the main thread for hundreds of ms on
      // large repos (one HTML string built, one insertAdjacentHTML
      // call, one querySelectorAll over the full table for click
      // wiring). Walk in CHUNK_SIZE batches and `await yieldToMain()`
      // between each so user input (drawer open, tab switch,
      // scrolling) stays responsive during the expansion.
      //
      // Small expansions render synchronously — the per-yield cost
      // (~0.5-2 ms message-channel round-trip + an extra paint cycle)
      // exceeds the gain when only a few chunks would run. The chunked
      // path is for the genuine "Show all on a 5000-row table" case.
      const CHUNK_SIZE = 50;
      const SYNC_THRESHOLD = 200;
      const totalEnd = Math.min(renderedRows + count, filteredView.length);
      const remaining = totalEnd - renderedRows;
      if (remaining <= SYNC_THRESHOLD) {
        await renderPageChunk(tbody, totalEnd);
        refreshActions();
        return;
      }
      while (renderedRows < totalEnd) {
        const next = Math.min(renderedRows + CHUNK_SIZE, totalEnd);
        await renderPageChunk(tbody, next);
        if (renderedRows < totalEnd) {
          // Only yield between chunks, not after the final one — the
          // caller's continuation (refreshActions) can run inline.
          await yieldToMain();
        }
      }
      refreshActions();
    }

    function renderPageChunk(tbody, next) {
      var html = '';
      for (var i = renderedRows; i < next; i++) {
        const r = filteredView[i];
        // MI cell: number + DaisyUI band badge (success / warning /
        // error) when mi_rank is finite. Empty when language is
        // unsupported by codelore-rca. The colour-coded pill carries
        // the top/mid/bottom-quartile triad accessibly for screen
        // readers and themably through DaisyUI's `--color-success` /
        // `--color-warning` / `--color-error` tokens.
        let miCell = '';
        if (typeof r.mi === 'number' && isFinite(r.mi)) {
          let bandBadge = '';
          if (typeof r.mi_rank === 'number' && isFinite(r.mi_rank)) {
            // Complete class-name literals (not string-concatenation)
            // so the Tailwind v4 pruner can see each variant during
            // `@source` scan of widgets.js — `'badge-' + kind` would
            // hide the suffix from the static scan and the variants
            // would drop out of the compiled CSS bundle.
            if (r.mi_rank >= 0.75) {
              bandBadge = ' <span class="badge badge-success badge-sm" title="MI band: High">High</span>';
            } else if (r.mi_rank >= 0.25) {
              bandBadge = ' <span class="badge badge-warning badge-sm" title="MI band: Mid">Mid</span>';
            } else {
              bandBadge = ' <span class="badge badge-error badge-sm" title="MI band: Low">Low</span>';
            }
          }
          miCell = r.mi.toFixed(1) + bandBadge;
        }
        // AI cell: percentage rendered as X% (rounded — table is dense,
        // decimal point would crowd). Wrapped in a DaisyUI outline
        // badge so the AI-attribution signal reads consistently with
        // the MI band badge above.
        const aiCell = (typeof r.ai_pct === 'number' && isFinite(r.ai_pct))
          ? '<span class="badge badge-outline badge-sm">' + Math.round(r.ai_pct) + '%</span>'
          : '';
        // `data-primary-author` lets the off-boarding effect (set up
        // below) toggle a `.hotspot-row-departed` class on rows whose
        // primary author is in `$store.scenario.departed` — the same
        // reactive signal the keyboard-accessible file list uses.
        // Lookup pulls from `_codelorePrimaryAuthorByPath`, populated
        // once at boot.
        const rowAuthor = (window._codelorePrimaryAuthorByPath || {})[r.path] || '';
        html += '<tr data-path="' + escapeHtml(r.path) + '" data-primary-author="' + escapeHtml(rowAuthor) + '" class="hotspot-row" style="cursor:pointer">' +
          '<td class="path">' + escapeHtml(r.path) + '</td>' +
          '<td class="num">' + (r.revisions != null ? r.revisions : '') + '</td>' +
          '<td class="num">' + fmtNumber(r.cognitive, { decimals: 0 }) + '</td>' +
          '<td class="num">' + fmtNumber(r.cognitive_health, { decimals: 1 }) + '</td>' +
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
        wireRowKbActivation(newRows[k]);
      }
      return Promise.resolve();
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
      // DaisyUI `btn btn-outline btn-sm` matches the dashboard's
      // button vocabulary (theme toggle, drawer close). Inline
      // `.table-actions button { ... }` rules in the `<style>` block
      // are no-op'd by this — the DaisyUI utility classes win
      // specificity now that we declare them explicitly.
      showNext.className = 'btn btn-outline btn-sm';
      showNext.textContent = 'Show next ' + next;
      // Element-scoped transition on `container` — the rest of the
      // dashboard stays interactive while the table animates.
      showNext.addEventListener('click', function () {
        startViewTransition(function () { renderNextPage(PAGE_SIZE); }, container);
      });
      actionsEl.appendChild(showNext);
      if (more > PAGE_SIZE) {
        const showAll = document.createElement('button');
        showAll.type = 'button';
        showAll.className = 'btn btn-outline btn-sm';
        showAll.textContent = 'Show all (' + more + ' more)';
        showAll.addEventListener('click', function () {
          startViewTransition(function () { renderNextPage(Infinity); }, container);
        });
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
      // Mirror the local filterText into the Alpine `filter` store so
      // any other widget that subscribes via `Alpine.effect(...)` sees
      // the live value. Wrapped in `window.Alpine` guard so the page
      // still works if Alpine fails to load (e.g. user disabled JS
      // through a content-security-policy header).
      if (window.Alpine) {
        window.Alpine.store('filter').set(filterText);
      }
      if (debounceTimer) clearTimeout(debounceTimer);
      debounceTimer = setTimeout(rerender, 80);
    });

    // Seed the input from the persisted Alpine store on first render
    // so a page reload (e.g. `--embed` step-summary roundtrip) brings
    // the user's last filter back. The store value comes from
    // localStorage via the Alpine persist plugin.
    if (window.Alpine) {
      const persisted = window.Alpine.store('filter').text;
      if (persisted && !filterEl.value) {
        filterEl.value = persisted;
        filterText = persisted;
      }
    }

    // Initial render.
    rerender();

    // Cross-widget selection: highlight the row for the selected path (if
    // it's on the current page); a null selection clears all row highlights.
    // Rows are rebuilt on sort/filter/paginate, so query the live DOM each
    // time rather than caching nodes.
    window._codeloreRegisterSelectionListener('hotspot-table', function (selectedPath) {
      const tbody = document.getElementById('hotspot-tbody');
      if (!tbody) return;
      const rows = tbody.querySelectorAll('tr');
      for (var i = 0; i < rows.length; i++) {
        const rowPath = rows[i].getAttribute('data-path');
        const isSel = !!selectedPath && rowPath === selectedPath;
        rows[i].classList.toggle('!bg-base-300', isSel);
        // Mark the selected row for assistive tech, not just visually.
        // aria-current is removed (not set to 'false') on non-selected rows
        // so only one row ever carries the state.
        if (isSel) {
          rows[i].setAttribute('aria-current', 'true');
        } else {
          rows[i].removeAttribute('aria-current');
        }
      }
    });

    // Cross-widget quadrant brush: emphasise every row whose path is in the
    // brushed set (distinct from the single-selection `!bg-base-300` — brush
    // = context, selection = focus; a row can carry both). Rebuilt on
    // sort/filter/paginate like the selection highlight, so it re-applies on
    // the next brush change (same transient-drop behaviour as selection).
    window._codeloreRegisterBrushListener('hotspot-table', function (cell, paths) {
      const tbody = document.getElementById('hotspot-tbody');
      if (!tbody) return;
      const set = new Set(paths || []);
      const rows = tbody.querySelectorAll('tr');
      for (var i = 0; i < rows.length; i++) {
        const p = rows[i].getAttribute('data-path');
        rows[i].classList.toggle('hotspot-row-brushed', !!p && set.has(p));
      }
    });
  }


