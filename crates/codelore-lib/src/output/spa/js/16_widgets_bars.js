  // ─── §A.5  Widget: banded share bars + effort dot strip ─────────────
  //
  // Renders two 100% stacked horizontal bars (LOC share and churn share
  // per code-health band) plus a 20-dot effort strip where each dot
  // represents 5% of trailing-window churn. All HTML/CSS — no ECharts.
  //
  // Band colours are emitted as `var(--color-*)` references in inline
  // styles, so the browser re-resolves them on theme swap without any
  // JS rerender (the widget registers rerender: false).
  // Accessibility: role="img" + aria-label on every bar and the dot strip;
  // percentage text labels inside segments serve as non-colour redundant
  // cues (WCAG 1.4.1 — Use of Color).
  //
  // Caption: "X% of the last {window} days' changes landed in red code"
  // with Wilson 95% CI expressed as a title/tooltip attribute.
  function renderShareBars(rows, opts) {
    var mount = document.getElementById('widget-share-bars-body');
    if (!mount) return;
    if (!rows || !rows.length) {
      mount.innerHTML =
        '<p class="text-base-content/50 text-sm">No effort-exposure data available.</p>';
      return;
    }

    var BAND_ORDER = ['red', 'yellow', 'green'];
    var BAND_LABEL = { red: 'Red', yellow: 'Yellow', green: 'Green' };

    // Index rows by band for O(1) lookup.
    var byBand = {};
    for (var i = 0; i < rows.length; i++) {
      byBand[rows[i].band] = rows[i];
    }

    // ── Bar builder ────────────────────────────────────────────────────
    // Builds one 100%-stacked horizontal bar using valueFn(row) → %.
    // Segments narrower than 0.5% are hidden (invisible sliver).
    // Text labels appear inside segments ≥8% wide (non-colour cue).
    function buildBar(axisLabel, valueFn, ariaDesc) {
      var total = 0;
      for (var b = 0; b < BAND_ORDER.length; b++) {
        var r = byBand[BAND_ORDER[b]];
        total += r ? (valueFn(r) || 0) : 0;
      }
      var segments = '';
      for (var bi = 0; bi < BAND_ORDER.length; bi++) {
        var band = BAND_ORDER[bi];
        var row = byBand[band];
        var raw = row ? (valueFn(row) || 0) : 0;
        var pct = total > 0 ? (raw / total * 100) : 0;
        if (pct < 0.5) continue;
        var pctStr = pct.toFixed(1);
        segments +=
          '<div class="share-bar-segment"' +
              ' style="width:' + pct.toFixed(2) + '%;background:' + bandColor(band) + ';"' +
              ' title="' + BAND_LABEL[band] + ': ' + pctStr + '%">' +
            (pct >= 8
              ? '<span class="share-bar-label">' + BAND_LABEL[band] + ' ' + pctStr + '%</span>'
              : '') +
          '</div>';
      }
      return (
        '<div class="share-bar-row">' +
          '<span class="share-bar-axis-label">' + axisLabel + '</span>' +
          '<div class="share-bar-track" role="img" aria-label="' + ariaDesc + '">' +
            segments +
          '</div>' +
        '</div>'
      );
    }

    var locBar = buildBar(
      'LOC',
      function (r) { return r.loc_share_pct; },
      'Source lines of code share per health band: ' +
        BAND_ORDER.map(function (b) {
          var r = byBand[b]; return BAND_LABEL[b] + ' ' + (r ? r.loc_share_pct.toFixed(1) : '0') + '%';
        }).join(', ')
    );
    var churnBar = buildBar(
      'Churn',
      function (r) { return r.churn_share_pct; },
      'Churn share per health band in the trailing window: ' +
        BAND_ORDER.map(function (b) {
          var r = byBand[b]; return BAND_LABEL[b] + ' ' + (r ? r.churn_share_pct.toFixed(1) : '0') + '%';
        }).join(', ')
    );

    // ── Caption beneath the churn bar ─────────────────────────────────
    var captionHtml = '';
    var redRow = byBand['red'];
    if (redRow) {
      var redChurn = (redRow.churn_share_pct || 0).toFixed(1);
      var ciLo = ((redRow.commit_share_ci_low  || 0) * 100).toFixed(1);
      var ciHi = ((redRow.commit_share_ci_high || 0) * 100).toFixed(1);
      var windowStr = (opts && opts.window_days)
        ? 'the last ' + opts.window_days + ' days’'
        : 'recent';
      captionHtml =
        '<p class="share-bars-caption"' +
            ' title="Wilson 95 % CI on commit share: [' + ciLo + ' %, ' + ciHi + ' %]">' +
          '<strong>' + redChurn + '%</strong> of ' + windowStr + ' changes landed in red code' +
        '</p>';
    }

    // ── 20-dot effort strip ────────────────────────────────────────────
    // Each dot represents 5% of total window churn, coloured by band.
    // Dot counts: round(churn_share_pct / 5) per band; remainder (to
    // reach exactly 20) goes to the band with the largest raw share.
    var dotCounts = {};
    var dotSum = 0;
    var maxBand = null;
    var maxShare = -1;
    for (var di = 0; di < BAND_ORDER.length; di++) {
      var db = BAND_ORDER[di];
      var dr = byBand[db];
      var share = dr ? (dr.churn_share_pct || 0) : 0;
      var rounded = Math.round(share / 5);
      dotCounts[db] = rounded;
      dotSum += rounded;
      if (share > maxShare) { maxShare = share; maxBand = db; }
    }
    var remainder = 20 - dotSum;
    if (remainder !== 0 && maxBand !== null) {
      dotCounts[maxBand] = (dotCounts[maxBand] || 0) + remainder;
    }

    var dots = '';
    for (var dbi = 0; dbi < BAND_ORDER.length; dbi++) {
      var dotBand = BAND_ORDER[dbi];
      var count = Math.max(0, dotCounts[dotBand] || 0);
      for (var k = 0; k < count; k++) {
        dots +=
          '<span class="effort-dot"' +
              ' style="background:' + bandColor(dotBand) + ';"' +
              ' title="' + BAND_LABEL[dotBand] + ' band (5% churn per dot)">' +
          '</span>';
      }
    }
    var dotAriaLabel =
      (dotCounts['red'] || 0) + ' red, ' +
      (dotCounts['yellow'] || 0) + ' yellow, ' +
      (dotCounts['green'] || 0) + ' green — each dot = 5% of window churn';
    var dotStrip =
      '<div class="effort-dot-strip-wrap">' +
        '<span class="share-bar-axis-label">Effort</span>' +
        '<div class="effort-dot-strip" role="img" aria-label="' + dotAriaLabel + '">' +
          dots +
        '</div>' +
      '</div>';

    mount.innerHTML =
      '<div class="share-bars-container">' +
        locBar +
        churnBar +
        dotStrip +
        captionHtml +
      '</div>';
  }

  // ─── §18 Knowledge surfaces widget ────────────────────────────────────
  //
  // Renders three panels into #widget-knowledge-surfaces-body:
  //   1. Familiarity bullet bars — team familiarity % and islands % using
  //      bandFor() colour coding (same thresholds as code-health bands).
  //   2. Team-composition stacked bar — onboarded / experienced / veteran
  //      commit share as a proportional bar.
  //   3. Coordination table — top-10 files by tier desc then entropy desc,
  //      clickable to open the file-detail drawer.
  function renderKnowledgeSurfaces(famRows, teamRows, coordRows) {
    var mount = document.getElementById('widget-knowledge-surfaces-body');
    if (!mount) return;

    var html = '';

    // ── 1. Familiarity bullet bars ─────────────────────────────────────
    var fam = famRows && famRows.length ? famRows[0] : null;
    if (fam) {
      var famPct = typeof fam.familiarity_pct === 'number' ? fam.familiarity_pct : 0;
      var islPct = typeof fam.islands_pct === 'number' ? fam.islands_pct : 0;
      var activeAuthors = typeof fam.active_authors === 'number' ? fam.active_authors : '—';
      // Familiarity: higher is better → green ≥ 70, yellow ≥ 40, red < 40
      var famColor = bandColor(famPct >= 70 ? 'green' : famPct >= 40 ? 'yellow' : 'red');
      // Islands: lower is better → green ≤ 20 %, yellow ≤ 40 %, red > 40 %
      var islColor = bandColor(islPct <= 20 ? 'green' : islPct <= 40 ? 'yellow' : 'red');
      html +=
        '<div class="knowledge-familiarity-bars">' +
          '<div class="share-bar-row" title="Mean team familiarity with active files (' + fmtNumberFlex(famPct, 1) + '%)">' +
            '<span class="share-bar-axis-label">Familiarity</span>' +
            '<div class="share-bar-track">' +
              '<div class="share-bar-fill" style="width:' + Math.min(100, famPct) + '%;background:' + famColor + ';"></div>' +
            '</div>' +
            '<span class="share-bar-value">' + fmtNumberFlex(famPct, 1) + '%</span>' +
          '</div>' +
          '<div class="share-bar-row" title="Knowledge islands: files with no active owner (' + fmtNumberFlex(islPct, 1) + '% of files)">' +
            '<span class="share-bar-axis-label">Islands</span>' +
            '<div class="share-bar-track">' +
              '<div class="share-bar-fill" style="width:' + Math.min(100, islPct) + '%;background:' + islColor + ';"></div>' +
            '</div>' +
            '<span class="share-bar-value">' + fmtNumberFlex(islPct, 1) + '%</span>' +
          '</div>' +
          '<p class="knowledge-caption">Active authors: <strong>' + activeAuthors + '</strong>' +
            (fam.verdict ? ' — <em>' + escapeHtml(fam.verdict) + '</em>' : '') +
          '</p>' +
        '</div>';
    }

    // ── 2. Team-composition stacked bar ───────────────────────────────
    if (teamRows && teamRows.length) {
      var bucketColors = {
        onboarded:  'var(--color-info,    oklch(0.623 0.214 259.532))',
        experienced:'var(--color-success, oklch(0.753 0.152 163.216))',
        veteran:    'var(--color-primary, oklch(0.491 0.270 282.717))',
      };
      // Tenure mix is computed from the real per-author rows via their
      // `bucket` field. The `__summary__` carrier row (which packs percentage
      // strings into `bucket`) is skipped; share is the author-count fraction
      // per bucket, rendered in a fixed bucket order so the bar is deterministic.
      var BUCKET_ORDER = ['onboarded', 'experienced', 'veteran'];
      var bucketCounts = { onboarded: 0, experienced: 0, veteran: 0 };
      var realAuthors = 0;
      for (var ti = 0; ti < teamRows.length; ti++) {
        var tr = teamRows[ti];
        if (tr.author === '__summary__') continue;
        if (Object.prototype.hasOwnProperty.call(bucketCounts, tr.bucket)) {
          bucketCounts[tr.bucket] += 1;
          realAuthors += 1;
        }
      }
      if (realAuthors > 0) {
        var segments = '';
        var legend = '';
        for (var bi = 0; bi < BUCKET_ORDER.length; bi++) {
          var bucket = BUCKET_ORDER[bi];
          var count = bucketCounts[bucket];
          if (count === 0) continue;
          var share = (count / realAuthors) * 100;
          var color = bucketColors[bucket];
          segments +=
            '<div class="team-bar-segment" style="width:' + fmtNumberFlex(share, 1) + '%;background:' + color + ';"' +
              ' title="' + escapeHtml(bucket) + ': ' + fmtNumberFlex(share, 1) + '% of authors, ' + count + ' author(s)">' +
            '</div>';
          legend +=
            '<span class="team-bar-key" style="color:' + color + ';">' + escapeHtml(bucket) + '</span> ' +
            fmtNumberFlex(share, 1) + '% (' + count + ')  ';
        }
        html +=
          '<div class="team-composition-bar">' +
            '<div class="team-bar-track" role="img" aria-label="Team tenure distribution">' + segments + '</div>' +
            '<p class="knowledge-caption">' + legend.trim() + '</p>' +
          '</div>';
      }
    }

    // ── 3. Coordination table ──────────────────────────────────────────
    if (coordRows && coordRows.length) {
      var tierBadge = function (t) {
        var colors = { high: 'badge-error', medium: 'badge-warning', low: 'badge-info', single: 'badge-ghost' };
        return '<span class="badge badge-sm ' + (colors[t] || 'badge-ghost') + '">' + escapeHtml(t) + '</span>';
      };
      var rows = '';
      for (var ci = 0; ci < coordRows.length; ci++) {
        var cr = coordRows[ci];
        var name = (cr.path || '').split('/').pop();
        rows +=
          '<tr class="hover coord-row" data-path="' + escapeHtml(cr.path || '') + '">' +
            '<td class="coord-path" title="' + escapeHtml(cr.path || '') + '">' + escapeHtml(name) + '</td>' +
            '<td>' + tierBadge(cr.tier || 'single') + '</td>' +
            '<td>' + fmtNumberFlex(cr.fragmentation, 2) + '</td>' +
            '<td>' + fmtNumberFlex(cr.cochange_entropy, 3) + '</td>' +
          '</tr>';
      }
      html +=
        '<div class="table-container coordination-table">' +
          '<table class="table table-xs">' +
            '<thead><tr>' +
              '<th scope="col">File</th><th scope="col">Tier</th><th scope="col">Fragmentation</th><th scope="col">Entropy</th>' +
            '</tr></thead>' +
            '<tbody>' + rows + '</tbody>' +
          '</table>' +
        '</div>';
    }

    if (!html) {
      html = '<p class="muted-hint">No knowledge data — run with source files present to populate.</p>';
    }

    mount.innerHTML = html;

    // Wire coordination rows to the file-detail drawer for linked
    // brushing — same data-path + _codeloreShowDetail pattern the
    // knowledge-islands and improvements-feed rows use.
    var coordRowEls = mount.querySelectorAll('tr.coord-row');
    for (var cw = 0; cw < coordRowEls.length; cw++) {
      coordRowEls[cw].addEventListener('click', function (evt) {
        var p = evt.currentTarget.getAttribute('data-path');
        if (window._codeloreShowDetail) window._codeloreShowDetail(p);
      });
      coordRowEls[cw].style.cursor = 'pointer';
      wireRowKbActivation(coordRowEls[cw]);
    }
  }

