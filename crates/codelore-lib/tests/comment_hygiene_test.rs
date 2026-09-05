//! Guard: no internal finding/task IDs (an `F` or `T` followed by digits) and
//! no phase-number markers — a [`PHASE_KEYWORDS`] word joined to a number by
//! spaces, a hyphen, or an underscore — anywhere in `.rs`/`.sql` source:
//! comment, string literal, DDL, or file name.
//!
//! The ID vocabulary was `F`-plus-digits only for three audit cycles, so the
//! `T`-prefixed series stayed invisible. One of them reached a published
//! `codelore explain` **Citation** field, where it sat among real sources
//! ("DORA 2018 Accelerate", "Bird et al. 2011"), and a `Task`-numbered marker
//! promised integration tests that had long since been written. Widening the
//! rule is not free — see [`line_has_ticket_id`] for the one shape in this
//! tree that looks like a `T`-prefixed ID and is not one.
//!
//! Note that this file is scanned like any other, so the rules are described
//! by shape rather than by example; a literal ID written here as an
//! illustration would be a violation, which is the guard behaving correctly.
//!
//! Code comments (and user-facing strings) must describe the current contract
//! directly; audit and finding history lives only in `CHANGELOG.md` and the
//! findings report. A bare audit-ID or phase marker rots as work ships and
//! means nothing to a reader without the report. This test fails the gate if
//! any such token reappears, so the convention can't silently regress (it was
//! re-introduced repeatedly before this guard existed).
//!
//! Scope: `.rs` and `.sql` under `crates/codelore-(lib|cli)/(src|tests)`. The
//! `.sql` schema (`facts/schema_v1.sql`) is code too and once carried the same
//! markers. The vendored `codelore-rca` MPL fork is intentionally excluded — it
//! tracks upstream and is hands-off. `CHANGELOG.md`, the findings report, and
//! other Markdown are out of scope: those are the sanctioned homes for audit
//! IDs.
//!
//! Both checks scan the WHOLE line, so a marker is caught in a comment, a
//! string literal (an `anyhow::bail!` message, an assertion label), a
//! multi-line string continuation, or DDL alike. The keyword-plus-digit and
//! `F`-plus-digits shapes carry no false-positive risk over this source tree;
//! the `T` series does, and is narrowed by [`line_has_ticket_id`].
//! Tokenisation keeps `_` inside a token, so an underscored identifier or a
//! hex-ish literal is a single token and is not flagged; only a standalone
//! token of that shape is.
//!
//! Task IDs also can't hide in a FILE NAME, where no content scanner would
//! reach them — the scanned file stems are checked against the same rule.

use std::path::{Path, PathBuf};

/// Roots scanned, relative to the workspace root.
const SCANNED: &[&str] = &[
    "crates/codelore-lib/src",
    "crates/codelore-lib/tests",
    "crates/codelore-cli/src",
    "crates/codelore-cli/tests",
];

/// Individual files scanned alongside [`SCANNED`]. Manifests carry prose
/// comments of exactly the kind this guard polices, and they sit at crate root
/// rather than under `src`/`tests`, so no root above reaches them — neither by
/// path nor by extension.
///
/// `crates/codelore-rca/UPSTREAM.md` is here because that crate's manifest
/// sets `readme = "UPSTREAM.md"`: it is the text crates.io publishes, and the
/// doc guards do not reach it either (they scan `README.md` plus `docs/**`).
///
/// The vendored fork is included at *manifest* level and excluded at *source*
/// level, because the split that matters is provenance rather than crate.
/// `codelore-rca/Cargo.toml` is codelore-authored — our grammar pins, our
/// node-ID annotations, our lint decisions — while the `src/` tree beside it is
/// upstream MPL code that no root above scans. The upstream issue references
/// that manifest carries (`#528`, `#1183`) are outside every rule here anyway:
/// this guard bans `F`/`T`-prefixed IDs and `Plan`/`Task`/`DEEP` phase
/// markers, not bare `#`-prefixed numbers.
const SCANNED_FILES: &[&str] = &[
    "Cargo.toml",
    "crates/codelore-lib/Cargo.toml",
    "crates/codelore-cli/Cargo.toml",
    "crates/codelore-rca/Cargo.toml",
    "crates/codelore-rca/UPSTREAM.md",
];
/// `CARGO_MANIFEST_DIR` is `<root>/crates/codelore-lib`; two levels up is the
/// workspace root. Embedded at compile time, so it resolves under CI too.
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root two levels above crates/codelore-lib")
        .to_path_buf()
}

/// Collect `.rs` and `.sql` source files. SQL is included because the
/// fact-store schema is code and can carry the same banned phase markers.
fn collect_source_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return; // a missing root is fine — just nothing to scan
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_source_files(&path, out);
        } else if matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("rs" | "sql")
        ) {
            out.push(path);
        }
    }
}

/// True if `token` is an ID of `prefix` followed by one to three digits and
/// nothing else.
fn is_id_token(token: &str, prefix: u8) -> bool {
    let bytes = token.as_bytes();
    matches!(bytes.len(), 2..=4) && bytes[0] == prefix && bytes[1..].iter().all(u8::is_ascii_digit)
}

/// True if `token` is a finding/task ID: an `F` followed by one to three
/// digits and nothing else.
fn is_task_id(token: &str) -> bool {
    is_id_token(token, b'F')
}

/// True if the line carries a bare task-ID token anywhere — comment, string
/// literal, or DDL.
fn line_has_task_id(line: &str) -> bool {
    // Split on non-identifier chars so `_` stays part of a token (mirrors the
    // `\b` word boundary): an underscored identifier stays a single token and
    // is NOT flagged, while parenthesised, hyphen-joined, or slash-joined IDs
    // split into bare ID tokens that ARE flagged.
    line.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .any(is_task_id)
}

/// True if the file stem opens with a task-ID segment (`f69_window_spike`).
/// Stems are `snake_case`, so the leading `_`-delimited segment is the only
/// place the prefix convention puts one, and it is the one position no
/// content scan can reach. Matched case-insensitively because file names are
/// lowercase; a stem legitimately opening with a float-width segment
/// (`f64_…`) would need renaming or an exemption here, which no file in this
/// tree currently requires.
fn stem_opens_with_task_id(stem: &str) -> bool {
    let head = stem.split('_').next().unwrap_or(stem);
    is_task_id(&head.to_ascii_uppercase())
}

/// Capitalised keywords that name a development phase or audit pass.
const PHASE_KEYWORDS: &[&str] = &["Plan", "Task", "DEEP"];

/// True if the line carries a phase-number marker: one of [`PHASE_KEYWORDS`]
/// at a word boundary, followed by optional spaces or a single `-`/`_`, then
/// an ASCII digit. These name development history (the sequence a feature
/// shipped in), not the current contract — the same banned class as finding
/// IDs. Scanned over the whole line so comment, string-literal, and DDL
/// markers are all caught (see the module doc for why a whole-line scan is
/// safe for these shapes but not for the `T` series).
fn line_has_plan_marker(line: &str) -> bool {
    PHASE_KEYWORDS
        .iter()
        .any(|keyword| line_has_keyword_number(line, keyword))
}

/// True if `line` carries `keyword` at a word boundary, followed by optional
/// spaces or a single `-`/`_` joiner, then an ASCII digit.
fn line_has_keyword_number(line: &str, keyword: &str) -> bool {
    let bytes = line.as_bytes();
    line.match_indices(keyword).any(|(start, _)| {
        // Word boundary before the keyword so a longer identifier ending in
        // the keyword (e.g. inside a path segment) doesn't false-match.
        if start > 0 && (bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'_') {
            return false;
        }
        let rest = line[start + keyword.len()..].trim_start_matches(' ');
        // A single `-` or `_` may join the keyword to its number. Without
        // this the hyphenated form is invisible to every rule here: the
        // token scans split it at the hyphen into a bare word and a bare
        // digit, neither of which is an ID, and the keyword scan stops at a
        // separator it does not expect.
        let rest = rest.strip_prefix(['-', '_']).unwrap_or(rest);
        rest.starts_with(|c: char| c.is_ascii_digit())
    })
}

/// True if the line carries a bare `T`-prefixed task-ID token anywhere.
///
/// Same tokenisation as [`line_has_task_id`], with one addition: `}` is kept
/// *inside* a token rather than splitting one. Every ISO-8601 timestamp in the
/// fixtures is built as `format!("…-{day:02}T10:00:00Z")`, so the separator
/// `T` sits immediately after a placeholder's closing brace; gluing that brace
/// to the token leaves `02}T10`, which is not a bare ID. That excludes
/// timestamps structurally rather than through an exemption list that would
/// rot as fixtures change.
///
/// No clone-type exemption is needed: the tree spells those `Type 1` / `Type
/// 2` / `Type 3`, which is also what the user-facing SARIF and dashboard
/// strings use, so the abbreviated form is not domain vocabulary here and no
/// number has to be carved out of the rule.
fn line_has_ticket_id(line: &str) -> bool {
    line.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '}'))
        .any(|token| is_id_token(token, b'T'))
}

fn scanned_files() -> Vec<PathBuf> {
    let root = workspace_root();
    let mut files = Vec::new();
    for rel in SCANNED {
        collect_source_files(&root.join(rel), &mut files);
    }
    for rel in SCANNED_FILES {
        let path = root.join(rel);
        // Named files are asserted to exist rather than skipped when missing.
        // A directory root that moves scans nothing and trips the emptiness
        // check below; a named file that moves would silently stop being
        // scanned while every test stayed green — the guard would go inert
        // exactly where its coverage was most deliberate.
        assert!(
            path.is_file(),
            "{rel} is listed for scanning but does not exist — the guard's \
             file list has drifted from the tree"
        );
        files.push(path);
    }
    assert!(
        !files.is_empty(),
        "scanned zero source files — source-path resolution is broken"
    );
    files
}

#[test]
fn no_task_id_references_in_code() {
    let root = workspace_root();
    let files = scanned_files();

    let mut violations = Vec::new();
    for file in &files {
        let rel = file.strip_prefix(&root).unwrap_or(file);
        if let Some(stem) = file.file_stem().and_then(|s| s.to_str())
            && stem_opens_with_task_id(stem)
        {
            violations.push(format!("{}: task ID in the file name", rel.display()));
        }
        let text = std::fs::read_to_string(file).expect("read source file");
        for (line_idx, line) in text.lines().enumerate() {
            if line_has_task_id(line) || line_has_ticket_id(line) {
                violations.push(format!(
                    "{}:{}: {}",
                    rel.display(),
                    line_idx + 1,
                    line.trim()
                ));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "found {} finding/task-ID reference(s) in .rs/.sql source (comment, string, DDL, \
         or file name). Drop the ID and keep the rationale — audit history lives in \
         CHANGELOG.md and the findings report, not in the code:\n{}",
        violations.len(),
        violations.join("\n"),
    );
}

#[test]
fn no_plan_phase_markers_in_code() {
    let root = workspace_root();
    let files = scanned_files();

    let mut violations = Vec::new();
    for file in &files {
        let text = std::fs::read_to_string(file).expect("read source file");
        for (line_idx, line) in text.lines().enumerate() {
            if line_has_plan_marker(line) {
                let rel = file.strip_prefix(&root).unwrap_or(file);
                violations.push(format!(
                    "{}:{}: {}",
                    rel.display(),
                    line_idx + 1,
                    line.trim()
                ));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "found {} phase-number marker(s) in .rs/.sql source (comment, string, or DDL). \
         Describe the current state and drop the marker — which release a feature shipped \
         in is history for CHANGELOG.md, not the code:\n{}",
        violations.len(),
        violations.join("\n"),
    );
}

/// A guard that cannot fail is worth nothing, and the scans above pass by
/// finding nothing — the same thing a broken predicate does. Pin both
/// directions on the shapes that actually occur in this tree.
///
/// The sample IDs are assembled at runtime rather than written as literals,
/// because this file is scanned like any other: a literal would violate the
/// very rule under test.
#[test]
fn the_hygiene_predicates_discriminate() {
    let t = |n: u32| format!("T{n}");

    // Real IDs, in the punctuation they were actually written with: a
    // trailing colon, a parenthesised aside, a following word. Single-digit
    // IDs are included because no number is exempt — the clone analyses spell
    // their type names `Type 1` / `Type 2` / `Type 3`, so nothing in this tree
    // needs a low-numbered `T` token to mean something else.
    for line in [
        format!("// {}: an author is considered departed", t(8)),
        format!("// {} (foo): bar", t(42)),
        format!("//! ({}) emitter note", t(11)),
        format!("// {} regression guard", t(9)),
        format!("// {}+{} exact match", t(1), t(2)),
    ] {
        assert!(line_has_ticket_id(&line), "must flag a task ID: {line:?}");
    }

    // Shapes a bare token rule would destroy.
    for line in [
        // An ISO-8601 stamp: the separator is glued to the preceding brace,
        // so the token reads `02}T10` rather than a bare ID.
        r#"let d = format!("2026-01-{day:02}T10:00:00Z");"#.to_string(),
        // Identifiers keep `_`/alphanumerics attached, so neither boundary
        // opens or closes a standalone token.
        format!("// INT8_C and {}_suffix identifiers", t(9)),
    ] {
        assert!(
            !line_has_ticket_id(&line),
            "must NOT flag a non-ID shape: {line:?}"
        );
    }

    // The phase-marker rule covers both keywords, and needs the digit.
    assert!(line_has_plan_marker(&format!("// tracked in Plan {}", 6)));
    assert!(line_has_plan_marker(&format!("// see Task {} for more", 9)));
    assert!(
        !line_has_plan_marker("// the plan is documented in the roadmap"),
        "lowercase prose is not a marker"
    );
    assert!(
        !line_has_plan_marker("// Task list lives in the roadmap"),
        "the keyword without a number is not a marker"
    );

    // Every keyword against every joiner, so one handled and the others
    // missed cannot pass — which is how the hyphenated spelling survived
    // every cycle before this.
    for keyword in PHASE_KEYWORDS {
        for joiner in ['-', '_', ' '] {
            let line = format!("// {keyword}{joiner}{} under compat", 3);
            assert!(
                line_has_plan_marker(&line),
                "must flag a joined marker: {line:?}"
            );
        }
        assert!(
            !line_has_plan_marker(&format!("// {keyword}-driven review notes")),
            "the keyword joined to a word rather than a number is not a marker"
        );
    }

    // The original vocabulary still works.
    let f = format!("F{}", 12);
    assert!(line_has_task_id(&format!("// {f}: the original shape")));
    assert!(
        !line_has_task_id(&format!("// _{f} stays an identifier")),
        "an underscored identifier is one token and is not an ID"
    );
}
