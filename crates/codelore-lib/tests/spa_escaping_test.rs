//! Guard: repository strings reaching an HTML sink in the SPA are escaped.
//!
//! The dashboard embeds its data as JSON and every widget builds markup by
//! string concatenation, so any repository-derived string — a path, a module
//! name, an author — is one missing `escapeHtml` away from executing in the
//! viewer's browser. Path names may legally contain `<` and `>` on Linux and
//! macOS, git tracks them verbatim, and the emitter's only JSON defence is
//! `"</"` → `"<\\/"`, which prevents `</script>` breakout and nothing else:
//! after `JSON.parse` the string carries its metacharacters intact, and the
//! next `innerHTML` concatenation is a fresh injection point that the
//! transport-level fix has no jurisdiction over. Escaping has to happen at
//! the sink, which is why the house convention is `escapeHtml` there.
//!
//! The class recurs. One widget built an `onclick` by concatenating row data
//! into an attribute; three cycles later a different widget concatenated
//! module paths into two chart tooltips — same defect, new file, because
//! nothing enforced the convention. This guard is the enforcement.
//!
//! What it checks: in any statement that also builds markup, an accessor
//! naming a repository-derived string must sit inside `escapeHtml(...)`.
//! Numeric fields are not listed — they cannot carry markup — and the
//! accessor list is derived from the JSON payload's string fields rather
//! than from an exemption list, so it does not rot as widgets change.
//!
//! What it does not check: markup assembled across statement boundaries, or
//! a field added to the payload without being added below. It is a
//! convention guard, not a taint tracker; the statement of its limits is
//! part of the guard.

use std::path::Path;

/// Every widget source `output::spa` concatenates, read from the directory
/// rather than listed here. A list would have to be edited twice — once
/// beside the emitter, once beside the guard — and the edit that gets
/// forgotten is the second one, leaving a new widget unscanned in exactly
/// the case this guard exists for: a file that never adopted the convention.
fn widget_sources() -> Vec<(String, String)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/output/spa/js");
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("js") {
                continue;
            }
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_owned();
            let src = std::fs::read_to_string(&path).expect("read widget source");
            out.push((name, src));
        }
    }
    out.sort();
    assert!(
        !out.is_empty(),
        "scanned zero widget sources — source-path resolution is broken"
    );
    out
}

/// Accessors whose value is a repository-derived string: the string fields
/// of the SPA JSON payload, the two chart-library carriers (`p.name`,
/// `order[...]`) that receive them, and a few fields carried by analyses
/// the dashboard does not embed today — cheap cover for the day one is.
///
/// Matching is by prefix, so `.entity` covers `.entity_a` and `.entity_b`:
/// a suffixed variant lands at the same byte offset, is judged from that
/// same offset, and so can only report the one defect twice.
const RAW_STRING_ACCESSORS: &[&str] = &[
    ".path",
    ".entity",
    ".author",
    ".canonical_author",
    ".source",
    ".target",
    ".module",
    ".tag",
    ".name",
    "order[",
];

/// Markers that a statement is building markup rather than plain text.
/// Written without the entity-terminating `;` on purpose: statements are
/// split on `;`, which would otherwise cut every entity in half and make
/// the markers unmatchable.
const HTML_MARKERS: &[&str] = &[
    "innerHTML",
    "<br",
    "<div",
    "<span",
    "<strong",
    "<p ",
    "&rarr",
    "&harr",
    "&middot",
    "title=\"",
];

/// Statement-ish slices. Markup here is built by concatenation terminated
/// by `;`, so splitting there keeps a multi-line `return` whole while
/// separating unrelated code.
///
/// HTML entities end in `;` too. Splitting on those would cut a statement
/// at `&rarr;` — severing it from the very marker that identifies it as
/// markup, and hiding every accessor after the entity. So a `;` that closes
/// an entity is not a statement boundary.
/// Each slice is paired with its byte offset in `src`, so a violation can
/// name the line in the file rather than the line within the statement —
/// the latter reads like a file line and points hundreds of lines away.
fn statements(src: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut start = 0;
    for (i, _) in src.match_indices(';') {
        let head = src[..i].trim_end_matches(|c: char| c.is_ascii_alphanumeric());
        // `head.len() < i` means at least one name character was trimmed;
        // without it a bare `&;` would read as an entity and never split.
        let closes_entity = head.len() < i && head.ends_with('&');
        if !closes_entity {
            out.push((start, &src[start..i]));
            start = i + 1;
        }
    }
    out.push((start, &src[start..]));
    out
}

/// What precedes the member expression containing `pos`, with the
/// expression's own identifier chain walked off.
fn preceding(stmt: &str, pos: usize) -> &str {
    let head = stmt[..pos]
        .rfind(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '$' || c == '.'))
        .map_or(0, |i| i + 1);
    stmt[..head].trim_end()
}

/// Whether the expression at `pos` is already wrapped in `escapeHtml(...)`.
fn is_escaped(stmt: &str, pos: usize) -> bool {
    preceding(stmt, pos).ends_with("escapeHtml(")
}

/// Whether the expression at `pos` is a subscript — `moduleRole[p.name]`
/// looks up a role by path, so the path is a key, not rendered output.
/// Escaping it would break the lookup rather than secure it.
fn is_lookup_key(stmt: &str, pos: usize) -> bool {
    preceding(stmt, pos).ends_with('[')
}

/// Unescaped raw-string accessors inside markup-building statements.
fn unescaped_sinks(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (offset, stmt) in statements(src) {
        if !HTML_MARKERS.iter().any(|m| stmt.contains(m)) {
            continue;
        }
        for accessor in RAW_STRING_ACCESSORS {
            for (at, _) in stmt.match_indices(accessor) {
                if !is_escaped(stmt, at) && !is_lookup_key(stmt, at) {
                    let line = src[..offset + at].matches('\n').count() + 1;
                    out.push(format!("{line}: {accessor}"));
                }
            }
        }
    }
    out
}

#[test]
fn no_widget_concatenates_repository_strings_into_markup_unescaped() {
    let mut violations = Vec::new();
    for (name, src) in widget_sources() {
        for hit in unescaped_sinks(&src) {
            violations.push(format!("  {name}:{hit}"));
        }
    }

    assert!(
        violations.is_empty(),
        "{} SPA sink(s) interpolate a repository-derived string into markup \
         without `escapeHtml`:\n{}\n\n\
         Repository paths may contain `<` and `>`; they reach the browser \
         verbatim because the emitter escapes only `</` in the JSON payload. \
         An unescaped concatenation into `innerHTML` — including the return \
         value of an ECharts function formatter, which is inserted as markup \
         rather than filtered like a `{{b}}` template — executes whatever the \
         analysed repository put in that path.\n\n\
         Wrap the value in `escapeHtml(...)`, the helper every other widget \
         already uses.",
        violations.len(),
        violations.join("\n"),
    );
}

#[test]
fn the_guard_catches_the_shape_it_exists_for() {
    // The real check passes when the tree is clean, which is also what a
    // broken matcher looks like. Pin it against the defect it was written
    // for — the architecture tooltips, in their pre-fix form — and against
    // the fixed form, so neither a vacuous pass nor a false positive can
    // hide. An earlier draft of this matcher required a `+` before the
    // accessor and allowed only one member segment; it reported zero
    // violations on the vulnerable code below.
    let vulnerable =
        "return 'Imports: ' + p.data.source + ' &rarr; ' + p.data.target + ' (' + n + ')';";
    assert_eq!(
        unescaped_sinks(vulnerable).len(),
        2,
        "must flag both unescaped edge endpoints"
    );

    let vulnerable_leading = "return p.name + '<br/>role: ' + role;";
    assert_eq!(
        unescaped_sinks(vulnerable_leading).len(),
        1,
        "must flag an accessor that opens the expression, with no `+` before it"
    );

    let vulnerable_index = "return order[r] + ' &rarr; ' + order[c] + '<br/>' + v;";
    assert_eq!(
        unescaped_sinks(vulnerable_index).len(),
        2,
        "must flag indexed axis labels"
    );

    let fixed =
        "return 'Imports: ' + escapeHtml(p.data.source) + ' &rarr; ' + escapeHtml(p.data.target);";
    assert_eq!(
        unescaped_sinks(fixed).len(),
        0,
        "must accept the escaped form"
    );

    // Markup-free statements are out of scope even when they carry paths,
    // or every data-plumbing line in the file would be a violation.
    let not_markup = "const rm = modulePath(rr.path, chosenDepth);";
    assert_eq!(
        unescaped_sinks(not_markup).len(),
        0,
        "a statement that builds no markup is not a sink"
    );

    // A path used as a subscript is a key, not rendered output. Escaping it
    // would change what is looked up rather than secure anything.
    let lookup = "return escapeHtml(p.name) + '<br/>role: ' + (moduleRole[p.name] || 'periphery');";
    assert_eq!(
        unescaped_sinks(lookup).len(),
        0,
        "an accessor inside a subscript is a lookup key, not a sink"
    );
}
