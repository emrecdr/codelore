//! Guard: no internal finding/task IDs (an `F` followed by digits) in `.rs`
//! code comments, and no `Plan <N>` phase markers anywhere in `.rs`/`.sql`
//! source — comment, string literal, or DDL.
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
//! Two checks, deliberately asymmetric in reach:
//!   * `Plan <N>` phase markers are scanned over the WHOLE line, so they are
//!     caught in comments, string literals (`anyhow::bail!("… Plan N")`), and
//!     multi-line string continuations alike. The `Plan`+digit shape is
//!     specific enough that a whole-line scan carries no false-positive risk.
//!   * `F<NN>` task IDs stay comment-scoped. Bare `F<NN>` tokens appear
//!     legitimately in test fixtures and assertion labels (git config names,
//!     regression-message prefixes), so scanning string literals for them would
//!     false-fire. Those string/filename-embedded test labels are a separate,
//!     broader hygiene item, tracked in the findings report — not this guard.

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

fn is_sql(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()) == Some("sql")
}

/// The comment region of a line: text from the first line-comment delimiter
/// (`//` for Rust, `--` for SQL), or the whole line when it is a block-comment
/// continuation (`*` / `/*`). `None` when the line carries no comment.
fn comment_region(line: &str, sql: bool) -> Option<&str> {
    let delim = if sql { "--" } else { "//" };
    if let Some(idx) = line.find(delim) {
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

fn comment_has_task_id(line: &str, sql: bool) -> bool {
    let Some(region) = comment_region(line, sql) else {
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
fn no_task_id_references_in_code_comments() {
    let root = workspace_root();
    let files = scanned_files();

    let mut violations = Vec::new();
    for file in &files {
        let sql = is_sql(file);
        let text = std::fs::read_to_string(file).expect("read source file");
        for (line_idx, line) in text.lines().enumerate() {
            if comment_has_task_id(line, sql) {
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
