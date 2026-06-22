//! Static-HTML report emitter (T11). One self-contained HTML file per
//! analysis run — embedded CSS + vanilla JS, no external CDN, no
//! framework dependency.
//!
//! ## Design choices
//!
//! - **Single-file output**: the entire report is one HTML file with
//!   inline CSS and JS. Works offline, attaches cleanly to GitHub
//!   Actions artifacts, opens in any browser. No CDN dependency.
//! - **JSON-data embedding**: the analysis rows are serialised to JSON
//!   inside a `<script type="application/json">` block. The vanilla JS
//!   reads that block to render the table — no separate data file.
//! - **Generic over `Serialize`**: any analysis row type that implements
//!   `serde::Serialize` (every existing row type does) can be rendered
//!   with one writer function. No per-analysis duplication.
//! - **No build-time template engine**: simple `&'static str` template
//!   with `{{TOKEN}}` placeholders. We don't introduce askama / tera /
//!   handlebars as dependencies.
//! - **Print-friendly**: `@media print` CSS rules collapse the
//!   interactive controls so the report can be saved as PDF for
//!   sharing with non-technical stakeholders.
//!
//! ## What's emitted
//!
//! - A header with the analysis name + repo path + run timestamp.
//! - A scrollable table with sortable column headers (click to sort).
//! - A simple text-filter input that filters rows by any column.
//! - Footer with provenance signature (codelore version, schema version).
//!
//! Sample output: ~50 KB for a 100-row report. Well under the 200 KB
//! soft cap for clean GitHub Actions artifact attachment.

use std::io::Write;

use crate::{CodeLoreError, Result};

/// Render any analysis row collection as a self-contained HTML report.
/// `rows` is serialized to JSON and embedded; the in-template JS reads
/// it to render the table at page load.
///
/// `title` is shown in the `<title>` tag and as the H1 header.
/// `repo_path` and `generated_at` are shown in the metadata block.
pub fn write_html<W: Write, T: serde::Serialize>(
    rows: &[T],
    w: &mut W,
    title: &str,
    repo_path: &str,
    generated_at: &str,
) -> Result<()> {
    let json = serde_json::to_string(rows)
        .map_err(|e| CodeLoreError::Output(format!("html json serialize: {e}")))?;
    // Escape characters that would break out of the `<script>` block. The
    // `</` sequence is the only one a JSON serializer can produce that's
    // dangerous in script context — `<` and `>` inside JSON strings are
    // not escaped by serde_json by default, so a row containing `</script>`
    // would terminate our embed block. Replace with the JSON-safe
    // `<\/script>` form.
    let json_safe = json.replace("</", "<\\/");

    // Single-pass templating to avoid the chained-`.replace()` pattern's
    // O(N×template) intermediate-string allocations on multi-megabyte
    // JSON payloads. See `output::template::substitute` for the impl.
    let title_escaped = html_escape(title);
    let repo_path_escaped = html_escape(repo_path);
    let generated_at_escaped = html_escape(generated_at);
    let row_count = rows.len().to_string();
    let html = crate::output::template::substitute(
        HTML_TEMPLATE,
        &[
            ("{{TITLE}}", &title_escaped),
            ("{{REPO_PATH}}", &repo_path_escaped),
            ("{{GENERATED_AT}}", &generated_at_escaped),
            ("{{ROW_COUNT}}", &row_count),
            ("{{DATA_JSON}}", &json_safe),
            ("{{CODELORE_VERSION}}", env!("CARGO_PKG_VERSION")),
        ],
    );

    w.write_all(html.as_bytes())
        .map_err(|e| CodeLoreError::Output(format!("html write: {e}")))?;
    Ok(())
}

/// Minimal HTML-attribute / text escape. Only handles the five chars
/// that can break out of HTML context: `& < > " '`. We don't use a
/// crate for this because we control all inputs (analysis name, repo
/// path, timestamps) — there's no XSS surface from user-supplied
/// content, just defensive belt-and-braces.
fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

/// The HTML template. Tokens (`{{NAME}}`) are substituted at render time.
/// CSS variables `--cl-*` make theming trivial; vanilla JS handles the
/// sort + filter interactivity without any framework dependency.
const HTML_TEMPLATE: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>CodeLore — {{TITLE}}</title>
<style>
  :root {
    --cl-bg: #0e1116;
    --cl-fg: #e6edf3;
    --cl-muted: #7d8590;
    --cl-accent: #58a6ff;
    --cl-warn: #f0883e;
    --cl-bad: #f85149;
    --cl-good: #3fb950;
    --cl-border: #30363d;
    --cl-card: #161b22;
    --cl-row-hover: #1f2630;
  }
  @media (prefers-color-scheme: light) {
    :root {
      --cl-bg: #ffffff;
      --cl-fg: #1f2328;
      --cl-muted: #59636e;
      --cl-accent: #0969da;
      --cl-warn: #bc4c00;
      --cl-bad: #cf222e;
      --cl-good: #1a7f37;
      --cl-border: #d1d9e0;
      --cl-card: #f6f8fa;
      --cl-row-hover: #eaeef2;
    }
  }
  * { box-sizing: border-box; }
  body {
    margin: 0;
    padding: 2rem;
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", "Helvetica Neue", Arial, sans-serif;
    font-size: 14px;
    line-height: 1.5;
    background: var(--cl-bg);
    color: var(--cl-fg);
  }
  header { margin-bottom: 1.5rem; }
  h1 {
    margin: 0 0 0.25rem 0;
    font-size: 1.5rem;
    color: var(--cl-accent);
  }
  .meta {
    color: var(--cl-muted);
    font-size: 0.85rem;
    display: flex;
    gap: 1rem;
    flex-wrap: wrap;
  }
  .meta span code {
    background: var(--cl-card);
    padding: 0.1rem 0.4rem;
    border-radius: 4px;
    font-size: 0.85em;
  }
  .controls {
    margin: 1rem 0;
    display: flex;
    gap: 0.5rem;
    align-items: center;
  }
  .controls input {
    flex: 1;
    max-width: 300px;
    padding: 0.4rem 0.6rem;
    background: var(--cl-card);
    color: var(--cl-fg);
    border: 1px solid var(--cl-border);
    border-radius: 6px;
    font-size: 0.85rem;
    font-family: inherit;
  }
  .controls input:focus {
    outline: none;
    border-color: var(--cl-accent);
  }
  .row-count {
    color: var(--cl-muted);
    font-size: 0.85rem;
  }
  table {
    width: 100%;
    border-collapse: collapse;
    background: var(--cl-card);
    border-radius: 8px;
    overflow: hidden;
    font-size: 0.85rem;
  }
  th, td {
    padding: 0.5rem 0.75rem;
    text-align: left;
    border-bottom: 1px solid var(--cl-border);
    vertical-align: top;
  }
  th {
    background: var(--cl-card);
    font-weight: 600;
    color: var(--cl-fg);
    cursor: pointer;
    user-select: none;
    position: sticky;
    top: 0;
    white-space: nowrap;
  }
  th:hover { color: var(--cl-accent); }
  th::after { content: ""; margin-left: 0.25rem; opacity: 0.5; }
  th.sort-asc::after  { content: "↑"; opacity: 1; color: var(--cl-accent); }
  th.sort-desc::after { content: "↓"; opacity: 1; color: var(--cl-accent); }
  tr:hover td { background: var(--cl-row-hover); }
  td code, td.code-cell {
    font-family: ui-monospace, SFMono-Regular, "SF Mono", Menlo, Consolas, monospace;
    font-size: 0.85em;
    color: var(--cl-fg);
  }
  td.num { text-align: right; font-variant-numeric: tabular-nums; }
  td.bad  { color: var(--cl-bad); font-weight: 600; }
  td.good { color: var(--cl-good); }
  td.warn { color: var(--cl-warn); }
  footer {
    margin-top: 2rem;
    color: var(--cl-muted);
    font-size: 0.75rem;
    text-align: center;
  }
  footer a { color: var(--cl-accent); text-decoration: none; }
  footer a:hover { text-decoration: underline; }
  .empty {
    padding: 3rem;
    text-align: center;
    color: var(--cl-muted);
    background: var(--cl-card);
    border-radius: 8px;
  }
  @media print {
    body { padding: 1rem; }
    .controls { display: none; }
    th { position: static; }
  }
</style>
</head>
<body>
<header>
  <h1>{{TITLE}}</h1>
  <div class="meta">
    <span>Repo: <code>{{REPO_PATH}}</code></span>
    <span>Generated: <code>{{GENERATED_AT}}</code></span>
    <span>Rows: <code>{{ROW_COUNT}}</code></span>
  </div>
</header>

<div class="controls">
  <input id="filter" type="text" placeholder="Filter rows by any column…" autocomplete="off">
  <span class="row-count" id="filteredCount"></span>
</div>

<div id="tableContainer"></div>

<footer>
  Generated by <a href="https://github.com/emrecdr/codelore">CodeLore</a>
  v{{CODELORE_VERSION}} —
  the behavioral-code-analysis CLI with published deterministic formulas.
</footer>

<script type="application/json" id="codelore-data">
{{DATA_JSON}}
</script>

<script>
(function () {
  const data = JSON.parse(document.getElementById('codelore-data').textContent);
  const container = document.getElementById('tableContainer');
  const filterInput = document.getElementById('filter');
  const filteredCountEl = document.getElementById('filteredCount');

  if (!data || data.length === 0) {
    container.innerHTML = '<div class="empty">No rows produced by this analysis. ' +
      'This may mean the thresholds filtered everything out, or the analysis is empty for this repo.</div>';
    return;
  }

  // Column inference from the first row. Preserves the key order of
  // the JSON object (which serde emits in struct-declaration order).
  const columns = Object.keys(data[0]);
  let sortKey = null;
  let sortDirection = 1; // 1 = asc, -1 = desc
  // Page-based rendering. Synchronously building 30k+ row
  // tables freezes the browser ("Page Unresponsive"). We render
  // PAGE_SIZE rows at a time and reveal more on "Show more" click.
  // Filtering / sorting always reset to page 1.
  const PAGE_SIZE = 500;
  let renderedRows = 0;
  let currentRows = data;

  function renderTable(rows) {
    currentRows = rows;
    renderedRows = 0;
    if (rows.length === 0) {
      container.innerHTML = '<div class="empty">No rows match the current filter.</div>';
      return;
    }
    let html = '<table><thead><tr>';
    for (const col of columns) {
      const sortClass = col === sortKey
        ? (sortDirection === 1 ? 'sort-asc' : 'sort-desc')
        : '';
      html += `<th class="${sortClass}" data-col="${escapeHTML(col)}">${escapeHTML(col)}</th>`;
    }
    html += '</tr></thead><tbody id="tbody"></tbody></table>';
    // "Show more" + "Show all" controls appear only when there are
    // more rows than fit on one page.
    html += '<div id="pageControls" style="margin:1rem 0;text-align:center;display:none;">' +
      '<button id="showMoreBtn" style="padding:0.4rem 0.8rem;background:var(--cl-card);' +
      'color:var(--cl-fg);border:1px solid var(--cl-border);border-radius:6px;cursor:pointer;' +
      'font-family:inherit;font-size:0.85rem;margin-right:0.5rem;">Show next 500</button>' +
      '<button id="showAllBtn" style="padding:0.4rem 0.8rem;background:var(--cl-card);' +
      'color:var(--cl-fg);border:1px solid var(--cl-border);border-radius:6px;cursor:pointer;' +
      'font-family:inherit;font-size:0.85rem;">Show all (slow)</button>' +
      '<span id="pageStatus" style="margin-left:1rem;color:var(--cl-muted);font-size:0.85rem;"></span>' +
      '</div>';
    container.innerHTML = html;
    renderNextPage();
    // Wire pagination controls.
    const moreBtn = document.getElementById('showMoreBtn');
    const allBtn = document.getElementById('showAllBtn');
    if (moreBtn) moreBtn.addEventListener('click', renderNextPage);
    if (allBtn) allBtn.addEventListener('click', () => renderNextPage(Infinity));
    // Wire sort-on-click for each header.
    for (const th of container.querySelectorAll('th')) {
      th.addEventListener('click', () => {
        const col = th.getAttribute('data-col');
        if (sortKey === col) {
          sortDirection = -sortDirection;
        } else {
          sortKey = col;
          sortDirection = 1;
        }
        applyFilterAndSort();
      });
    }
  }

  // Incrementally append `count` more rows (default
  // PAGE_SIZE). Uses a single innerHTML write per batch — building
  // 500 rows takes <50ms even on low-end devices, well below the
  // 100ms UI-freeze perception threshold.
  function renderNextPage(count) {
    if (count === undefined) count = PAGE_SIZE;
    const tbody = document.getElementById('tbody');
    if (!tbody) return;
    const start = renderedRows;
    const end = Math.min(currentRows.length, start + count);
    let html = '';
    for (let i = start; i < end; i++) {
      const row = currentRows[i];
      html += '<tr>';
      for (const col of columns) {
        const val = row[col];
        html += `<td class="${cellClass(col, val)}">${formatCell(val)}</td>`;
      }
      html += '</tr>';
    }
    tbody.insertAdjacentHTML('beforeend', html);
    renderedRows = end;
    const ctrl = document.getElementById('pageControls');
    const status = document.getElementById('pageStatus');
    if (ctrl) {
      if (renderedRows < currentRows.length) {
        ctrl.style.display = '';
        if (status) {
          status.textContent = `Showing ${renderedRows.toLocaleString()} of ` +
            `${currentRows.length.toLocaleString()} rows`;
        }
      } else {
        ctrl.style.display = 'none';
      }
    }
  }

  function escapeHTML(s) {
    return String(s).replace(/[&<>"']/g, c => ({
      '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;'
    }[c]));
  }

  function formatCell(v) {
    if (v === null || v === undefined) return '<span style="opacity:0.5">—</span>';
    if (typeof v === 'boolean') return v ? '✓' : '';
    if (typeof v === 'number') {
      if (Number.isInteger(v)) return v.toLocaleString();
      return v.toFixed(2);
    }
    return escapeHTML(v);
  }

  function cellClass(col, val) {
    const lc = col.toLowerCase();
    const classes = [];
    if (typeof val === 'number') classes.push('num');
    if (lc.includes('path') || lc.includes('entity') || lc.includes('file')) classes.push('code-cell');
    // Heuristic semantic styling for common risk columns.
    if ((lc.includes('at_risk') || lc === 'is_departed') && val === true) classes.push('bad');
    return classes.join(' ');
  }

  function applyFilterAndSort() {
    const filter = filterInput.value.trim().toLowerCase();
    let rows = data;
    if (filter) {
      rows = rows.filter(row =>
        columns.some(col => {
          const v = row[col];
          if (v === null || v === undefined) return false;
          return String(v).toLowerCase().includes(filter);
        })
      );
    }
    if (sortKey) {
      rows = rows.slice().sort((a, b) => {
        const av = a[sortKey], bv = b[sortKey];
        if (av === bv) return 0;
        if (av === null || av === undefined) return 1;
        if (bv === null || bv === undefined) return -1;
        if (typeof av === 'number' && typeof bv === 'number') {
          return (av - bv) * sortDirection;
        }
        return String(av).localeCompare(String(bv)) * sortDirection;
      });
    }
    renderTable(rows);
    filteredCountEl.textContent = filter
      ? `${rows.length} of ${data.length} rows`
      : '';
  }

  filterInput.addEventListener('input', applyFilterAndSort);
  applyFilterAndSort();
})();
</script>
</body>
</html>
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(serde::Serialize)]
    struct TestRow {
        name: String,
        count: u32,
    }

    #[test]
    fn write_html_embeds_data_and_metadata() {
        let rows = vec![
            TestRow {
                name: "first".into(),
                count: 1,
            },
            TestRow {
                name: "second".into(),
                count: 2,
            },
        ];
        let mut buf = Vec::new();
        write_html(
            &rows,
            &mut buf,
            "Test Analysis",
            "/path/to/repo",
            "2026-06-10T12:00:00Z",
        )
        .unwrap();
        let html = String::from_utf8(buf).unwrap();
        assert!(html.contains("Test Analysis"));
        assert!(html.contains("/path/to/repo"));
        assert!(html.contains("2026-06-10T12:00:00Z"));
        assert!(html.contains(r#""name":"first""#));
        assert!(html.contains(r#""count":2"#));
        assert!(html.contains("<!DOCTYPE html>"));
    }

    #[test]
    fn write_html_escapes_xss_in_metadata() {
        let rows: Vec<TestRow> = vec![];
        let mut buf = Vec::new();
        write_html(&rows, &mut buf, "<script>alert(1)</script>", "/safe", "now").unwrap();
        let html = String::from_utf8(buf).unwrap();
        // Raw `<script>alert(1)</script>` MUST NOT appear; escaped form must.
        assert!(!html.contains("<script>alert(1)</script>"));
        assert!(html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
    }

    #[test]
    fn write_html_escapes_script_terminator_in_json() {
        // A row whose value contains `</script>` would terminate the
        // embed block if not escaped. Verify the `</` → `<\/` rewrite
        // catches this.
        #[derive(serde::Serialize)]
        struct Evil {
            content: String,
        }
        let rows = vec![Evil {
            content: "</script><script>alert(1)</script>".into(),
        }];
        let mut buf = Vec::new();
        write_html(&rows, &mut buf, "x", "y", "z").unwrap();
        let html = String::from_utf8(buf).unwrap();
        // Raw `</script>` from the row data MUST be escaped inside JSON.
        // Only the TWO template-source `</script>` tags should remain
        // (the data-embed `<script type="application/json">...</script>`
        // and the interactive `<script>(function(){...})();</script>`).
        let script_close_count = html.matches("</script>").count();
        assert_eq!(
            script_close_count, 2,
            "exactly two literal `</script>` (data block + interactive); got {script_close_count}",
        );
        // The escaped form MUST be present.
        assert!(
            html.contains(r"<\/script>"),
            "row's `</script>` must be escaped as `<\\/script>` inside JSON embed",
        );
    }
}
