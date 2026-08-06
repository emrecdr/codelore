//! Guard: no internal finding/task IDs (an `F` followed by digits) and no
//! `Plan <N>` phase markers anywhere in `.rs`/`.sql` source — comment, string
//! literal, DDL, or file name.
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
//! string literal (`anyhow::bail!("… Plan N")`, an assertion label), a
//! multi-line string continuation, or DDL alike. Neither shape occurs
//! incidentally: `Plan`+digit and a standalone `F`+digits token are both
//! specific enough that a whole-line scan carries no false-positive risk over
//! this source tree. Tokenisation keeps `_` inside a token, so an identifier
//! such as `_F12` or a hex-ish `0xF12` is a single token and is not flagged;
//! only a standalone token of that shape is.
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

/// True if `token` is a finding/task ID: an `F` followed by one to three
/// digits and nothing else.
fn is_task_id(token: &str) -> bool {
    let bytes = token.as_bytes();
    matches!(bytes.len(), 2..=4) && bytes[0] == b'F' && bytes[1..].iter().all(u8::is_ascii_digit)
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

/// True if the line carries a phase-number marker: the capitalised word `Plan`
/// at a word boundary, directly followed by optional spaces then an ASCII
/// digit. These name development history (the sequence a feature shipped in),
/// not the current contract — the same banned class as finding IDs. Scanned
/// over the whole line so comment, string-literal, and DDL markers are all
/// caught (see the module doc for why this is safe here but not for `F<NN>`).
fn line_has_plan_marker(line: &str) -> bool {
    let bytes = line.as_bytes();
    let mut search_from = 0;
    while let Some(pos) = line[search_from..].find("Plan") {
        let start = search_from + pos;
        // Word boundary before the keyword so a longer identifier ending in
        // "Plan" (e.g. inside a path segment) doesn't false-match.
        let boundary_ok =
            start == 0 || !(bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'_');
        let mut j = start + 4;
        while j < bytes.len() && bytes[j] == b' ' {
            j += 1;
        }
        if boundary_ok && j < bytes.len() && bytes[j].is_ascii_digit() {
            return true;
        }
        search_from = start + 4;
    }
    false
}

fn scanned_files() -> Vec<PathBuf> {
    let root = workspace_root();
    let mut files = Vec::new();
    for rel in SCANNED {
        collect_source_files(&root.join(rel), &mut files);
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
            if line_has_task_id(line) {
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
