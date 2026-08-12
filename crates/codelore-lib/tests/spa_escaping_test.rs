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

/// Every widget source, in the order `output::spa` concatenates them.
const WIDGET_SOURCES: &[(&str, &str)] = &[
    (
        "00_setup_boot.js",
        include_str!("../src/output/spa/js/00_setup_boot.js"),
    ),
    (
        "10_helpers.js",
        include_str!("../src/output/spa/js/10_helpers.js"),
    ),
    (
        "12_drawer.js",
        include_str!("../src/output/spa/js/12_drawer.js"),
    ),
    (
        "14_widgets_summary.js",
        include_str!("../src/output/spa/js/14_widgets_summary.js"),
    ),
    (
        "16_widgets_bars.js",
        include_str!("../src/output/spa/js/16_widgets_bars.js"),
    ),
    (
        "20_hotspots.js",
        include_str!("../src/output/spa/js/20_hotspots.js"),
    ),
    (
        "30_coupling_trends.js",
        include_str!("../src/output/spa/js/30_coupling_trends.js"),
    ),
    (
        "40_architecture.js",
        include_str!("../src/output/spa/js/40_architecture.js"),
    ),
    (
        "50_calendar_xray.js",
        include_str!("../src/output/spa/js/50_calendar_xray.js"),
    ),
    (
        "90_toggles_utils.js",
        include_str!("../src/output/spa/js/90_toggles_utils.js"),
    ),
];

/// Accessors whose value is a repository-derived string. Taken from the
/// string fields of the SPA JSON payload plus the two chart-library
/// carriers (`p.name`, `order[...]`) that receive them.
const RAW_STRING_ACCESSORS: &[&str] = &[
    ".path",
    ".entity",
    ".entity_a",
    ".entity_b",
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
fn statements(src: &str) -> Vec<String> {
    let bytes = src.as_bytes();
    let mut out = Vec::new();
    let mut start = 0;
    for (i, b) in bytes.iter().enumerate() {
        if *b != b';' {
            continue;
        }
        let name_start = src[..i]
            .rfind(|c: char| !c.is_ascii_alphanumeric())
            .map_or(0, |j| j + 1);
        let closes_entity = name_start > 0 && name_start < i && bytes[name_start - 1] == b'&';
        if !closes_entity {
            out.push(src[start..i].to_owned());
            start = i + 1;
        }
    }
    out.push(src[start..].to_owned());
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
    for stmt in statements(src) {
        if !HTML_MARKERS.iter().any(|m| stmt.contains(m)) {
            continue;
        }
        for accessor in RAW_STRING_ACCESSORS {
            let mut from = 0;
            while let Some(rel) = stmt[from..].find(accessor) {
                let at = from + rel;
                if !is_escaped(&stmt, at) && !is_lookup_key(&stmt, at) {
                    let line = stmt[..at].lines().count();
                    out.push(format!("{accessor} (statement line ~{line})"));
                }
                from = at + accessor.len();
            }
        }
    }
    out
}

#[test]
fn no_widget_concatenates_repository_strings_into_markup_unescaped() {
    let mut violations = Vec::new();
    for (name, src) in WIDGET_SOURCES {
        for hit in unescaped_sinks(src) {
            violations.push(format!("  {name}: {hit}"));
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
