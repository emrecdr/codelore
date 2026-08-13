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
//! Numeric fields are not listed — they cannot carry markup.
//!
//! The accessor list is **curated, not derived**. The analyses carry
//! roughly seventy distinct `String` field names between them and this list
//! names a fraction: most of the rest are computed rather than
//! repository-derived — a band, a verdict, a trend, a date, a revision
//! hash — and cannot carry a metacharacter no matter what the analysed
//! repository contains. Curation is what keeps the list short enough to
//! read, and it is also the standing liability: a genuinely
//! repository-derived field is covered only if someone adds it. Two are
//! knowingly absent, because a single-letter accessor cannot work under
//! prefix matching — `.a` would swallow `.author`, `.added` and
//! `.arch_band` — so the function-coupling endpoints are unguarded should
//! they ever be rendered rather than used as lookup keys.
//!
//! What it does not check: markup assembled across statement boundaries —
//! including a repository string parked in a local by one statement and
//! rendered by the next — or a field added to the payload without being
//! added below. It is a convention guard, not a taint tracker; the
//! statement of its limits is part of the guard.

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
/// same offset, and so can only report the one defect twice. Suffix
/// compounds therefore need no entry of their own.
///
/// The `_`-prefixed entries cover the opposite shape, a qualifier-first
/// compound: `main_author` is a payload field rendered today, and `.author`
/// does not occur in it because the character before `author` is `_`.
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
    ".function",
    "order[",
    "_author",
    "_path",
    "_name",
];

/// Tag names the widgets emit. Checked against the token following a `<`
/// that opens a tag *inside a string literal* — see `builds_markup`.
const HTML_TAGS: &[&str] = &[
    "div", "span", "br", "strong", "p", "td", "tr", "th", "dt", "dd", "dl", "li", "ul", "ol", "h1",
    "h2", "h3", "h4", "h5", "code", "b", "em", "i", "a", "img", "option", "small", "label",
    "button", "svg", "table", "tbody", "thead",
];

/// Markup evidence that is not an opening tag. Written without the
/// entity-terminating `;` on purpose: statements are split on `;`, which
/// would otherwise cut every entity in half and make these unmatchable.
const HTML_ENTITIES: &[&str] = &["&rarr", "&harr", "&middot", "&nbsp", "title=\""];

/// Whether a statement builds markup rather than plain text.
///
/// A `<` only counts when it opens a known tag inside a string literal.
/// Both halves of that rule pay for themselves: without the literal
/// anchor, `evt.target` in a `for` loop's `j < n` reads as markup and every
/// accessor in the enclosing handler is flagged; without the tag-name
/// check, the literal `'<anonymous>'` — the placeholder for an unnamed
/// function in the X-ray widget — reads as an opening tag.
fn builds_markup(stmt: &str) -> bool {
    if stmt.contains("innerHTML") || HTML_ENTITIES.iter().any(|e| stmt.contains(e)) {
        return true;
    }
    stmt.match_indices('<').any(|(i, _)| {
        let before = stmt[..i].trim_end_matches(' ');
        if !before.ends_with('\'') && !before.ends_with('"') {
            return false;
        }
        let rest = &stmt[i + 1..];
        let rest = rest.strip_prefix('/').unwrap_or(rest);
        let len = rest
            .find(|c: char| !c.is_ascii_alphanumeric())
            .unwrap_or(rest.len());
        len > 0 && HTML_TAGS.contains(&&rest[..len])
    })
}

/// Statement-ish slices. Markup here is built by concatenation terminated
/// by `;`, so splitting there keeps a multi-line `return` whole while
/// separating unrelated code.
///
/// HTML entities end in `;` too. Splitting on those would cut a statement
/// at `&rarr;` — severing it from the very marker that identifies it as
/// markup, and hiding every accessor after the entity. So a `;` that closes
/// an entity is not a statement boundary.
///
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

/// Byte index where the identifier chain ending `s` begins.
///
/// Stepping back by the terminator's own width, rather than one byte, keeps
/// the index on a character boundary: the widgets carry `—` and `·`, and a
/// multi-byte terminator would otherwise split mid-character and panic the
/// guard instead of reporting on the file. Both callers need that property,
/// which is why the walk lives here instead of in each of them — a second
/// copy is a second place for the boundary step to be got wrong.
///
/// `dotted` decides whether `.` continues the chain: a member expression is
/// walked off whole (`p.data.source`), while a callee name stops at the dot
/// so the function's own name stays isolated.
fn ident_start(s: &str, dotted: bool) -> usize {
    s.char_indices()
        .rev()
        .find(|(_, c)| {
            !(c.is_ascii_alphanumeric() || *c == '_' || *c == '$' || (dotted && *c == '.'))
        })
        .map_or(0, |(i, c)| i + c.len_utf8())
}

/// What precedes the member expression containing `pos`, with the
/// expression's own identifier chain walked off.
fn preceding(stmt: &str, pos: usize) -> &str {
    stmt[..ident_start(&stmt[..pos], true)].trim_end()
}

/// Whether the expression at `pos` sits inside an `escapeHtml(...)` call.
///
/// Found by walking back to the innermost unclosed `(`, not by testing the
/// text immediately before the accessor. The difference is real: in
/// `escapeHtml(a.axisValueLabel || b.name)` a single call escapes *both*
/// operands, but only the first is adjacent to it — the immediate-prefix
/// test read `b.name` as a bare sink and reported a file that was correct.
fn is_escaped(stmt: &str, pos: usize) -> bool {
    let mut depth = 0usize;
    for (i, c) in stmt[..pos].char_indices().rev() {
        match c {
            ')' => depth += 1,
            '(' if depth == 0 => {
                let head = stmt[..i].trim_end();
                // Not `dotted`: the callee's own name is wanted, so a
                // qualified `a.escapeHtml(` must not read as `escapeHtml`.
                return &head[ident_start(head, false)..] == "escapeHtml";
            }
            '(' => depth -= 1,
            _ => {}
        }
    }
    false
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
        if !builds_markup(stmt) {
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

#[test]
fn the_guard_reaches_table_markup_and_qualifier_first_fields() {
    // Author names are the sharper vector of this class: a path needs `<`,
    // which only some filesystems allow, but `git commit --author` accepts
    // a quote verbatim, and author identity is rendered into attributes.
    // Both properties of the sink that carries it — markup built from
    // table/list tags, and a field named `main_author` — were once
    // invisible here, so each gets a case.
    let compound = "html += '<td>' + (r.main_author || '') + '</td>';";
    assert_eq!(
        unescaped_sinks(compound).len(),
        1,
        "a qualifier-first compound field is a sink; `.author` does not occur in `main_author`"
    );
    assert_eq!(
        unescaped_sinks("html += '<td>' + escapeHtml(r.main_author || '') + '</td>';").len(),
        0,
        "must accept the escaped compound field"
    );

    // A function name is parsed out of the analysed repository's source, so
    // it carries whatever that source put in an identifier position — the
    // same provenance as a path, and rendered in the same drawer.
    let parsed_identifier = "html += '<li><code>' + (f.function || '') + '</code></li>';";
    assert_eq!(
        unescaped_sinks(parsed_identifier).len(),
        1,
        "a function name parsed from the analysed source is repository-derived"
    );

    let suffix = "return '<div>' + p.entity_a + '</div>';";
    assert_eq!(
        unescaped_sinks(suffix).len(),
        1,
        "a suffix compound is covered by its base accessor, with no entry of its own"
    );

    // `escapeHtml(a || b)` escapes both operands with one call. Reading
    // only the text adjacent to the accessor reported the second as bare.
    let disjunction = "var html = '<b>' + escapeHtml(p.axisValueLabel || p.name) + '</b>';";
    assert_eq!(
        unescaped_sinks(disjunction).len(),
        0,
        "one escapeHtml call covers every operand inside its parentheses"
    );

    // A `<` that is a comparison, not a tag.
    let comparison = "if (a.name < b.name) { rank = 1; }";
    assert_eq!(
        unescaped_sinks(comparison).len(),
        0,
        "a less-than outside a string literal does not make a statement markup"
    );

    // A string literal shaped like a tag but naming no element.
    let placeholder = "node.children.push({ label: r.function || '<anonymous>', value: r.path });";
    assert_eq!(
        unescaped_sinks(placeholder).len(),
        0,
        "`<anonymous>` is a placeholder in the X-ray widget, not an opening tag"
    );
}
