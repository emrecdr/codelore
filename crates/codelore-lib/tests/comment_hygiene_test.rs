//! Guard: no internal finding/task IDs (an `F` followed by digits) in `.rs`
//! code comments.
//!
//! Code comments must describe the current contract directly; audit and
//! finding history lives only in `CHANGELOG.md` and the findings report. A
//! bare audit-ID token in a comment rots as findings close and means nothing
//! to a reader without the report. This test fails the gate if any such token
//! reappears in library or CLI comments, so the convention can't silently
//! regress (it was re-introduced repeatedly before this guard existed).
//!
//! Scope: `crates/codelore-(lib|cli)/(src|tests)`. The vendored
//! `codelore-rca` MPL fork is intentionally excluded — it tracks upstream and
//! is hands-off. `CHANGELOG.md`, the findings report, and other Markdown are
//! out of scope: those are the sanctioned homes for audit IDs.

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

fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return; // a missing root is fine — just nothing to scan
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rs_files(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

/// The comment region of a line: text from the first `//`, or the whole line
/// when it is a block-comment continuation (`*` / `/*`). `None` if the line
/// carries no comment. Line-based, matching the verification grep — a `//`
/// inside a string literal counts, but the codebase has no such case.
fn comment_region(line: &str) -> Option<&str> {
    if let Some(idx) = line.find("//") {
        return Some(&line[idx..]);
    }
    let trimmed = line.trim_start();
    (trimmed.starts_with('*') || trimmed.starts_with("/*")).then_some(line)
}

/// True if `token` is a finding/task ID: an `F` followed by one to three
/// digits and nothing else.
fn is_task_id(token: &str) -> bool {
    let bytes = token.as_bytes();
    matches!(bytes.len(), 2..=4) && bytes[0] == b'F' && bytes[1..].iter().all(u8::is_ascii_digit)
}

fn comment_has_task_id(line: &str) -> bool {
    let Some(region) = comment_region(line) else {
        return false;
    };
    // Split on non-identifier chars so `_` stays part of a token (mirrors the
    // `\b` word boundary): an underscored identifier stays a single token and
    // is NOT flagged, while parenthesised, hyphen-joined, or slash-joined IDs
    // split into bare ID tokens that ARE flagged.
    region
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .any(is_task_id)
}

/// True if the comment carries a phase-number marker: the capitalised word
/// `Plan` at a word boundary, directly followed by optional spaces then an
/// ASCII digit. These name development history (the sequence a feature shipped
/// in), not the current contract — the same banned class as finding IDs. A
/// whole-region scan is required because such a marker splits into two tokens,
/// so the per-token check above cannot see it.
fn comment_has_plan_marker(line: &str) -> bool {
    let Some(region) = comment_region(line) else {
        return false;
    };
    let bytes = region.as_bytes();
    let mut search_from = 0;
    while let Some(pos) = region[search_from..].find("Plan") {
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

#[test]
fn no_task_id_references_in_code_comments() {
    let root = workspace_root();
    let mut files = Vec::new();
    for rel in SCANNED {
        collect_rs_files(&root.join(rel), &mut files);
    }
    assert!(
        !files.is_empty(),
        "scanned zero .rs files — source-path resolution is broken"
    );

    let mut violations = Vec::new();
    for file in &files {
        let text = std::fs::read_to_string(file).expect("read source file");
        for (line_idx, line) in text.lines().enumerate() {
            if comment_has_task_id(line) {
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
        "found {} finding/task-ID reference(s) in code comments. Drop the ID and keep \
         the rationale — audit history lives in CHANGELOG.md and the findings report, \
         not in code comments:\n{}",
        violations.len(),
        violations.join("\n"),
    );
}

#[test]
fn no_plan_phase_markers_in_code_comments() {
    let root = workspace_root();
    let mut files = Vec::new();
    for rel in SCANNED {
        collect_rs_files(&root.join(rel), &mut files);
    }
    assert!(
        !files.is_empty(),
        "scanned zero .rs files — source-path resolution is broken"
    );

    let mut violations = Vec::new();
    for file in &files {
        let text = std::fs::read_to_string(file).expect("read source file");
        for (line_idx, line) in text.lines().enumerate() {
            if comment_has_plan_marker(line) {
                let rel = file.strip_prefix(&root).unwrap_or(file);
                violations.push(format!("{}:{}: {}", rel.display(), line_idx + 1, line.trim()));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "found {} phase-number marker(s) in code comments. Describe the current state \
         and drop the marker — which release a feature shipped in is history for \
         CHANGELOG.md, not the code comment:\n{}",
        violations.len(),
        violations.join("\n"),
    );
}
