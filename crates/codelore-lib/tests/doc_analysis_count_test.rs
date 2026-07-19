//! Guard: no tracked doc under `docs/` states a stale hard-coded "N
//! analyses" count that disagrees with the live `AnalysisName` registry.
//!
//! Reference docs describe the registry by shape ("the full analysis
//! registry, enumerated by `AnalysisName::all()`") rather than by a pinned
//! number, precisely because the registry grows. A hard-coded count left
//! behind after that growth is the same class of rot the project already
//! guards against for stale test counts and version numbers — this test
//! closes the same gap for analysis counts, so it can't silently regress.
//!
//! Scope: every `.md` file under `docs/`, excluding documents that
//! legitimately narrate a point-in-time count rather than a claim about
//! the current registry: `docs/superpowers/` (dated plans and specs),
//! `docs/RELEASING.md` (its "how we got here" section narrates past
//! release milestones, changelog-style), and `docs/maximum-feature-plan.md`
//! (marked fully shipped — a frozen plan that predates the
//! `docs/superpowers/` convention).

use std::path::{Path, PathBuf};

use codelore_lib::analysis::AnalysisName;

/// Path prefixes excluded from the scan (see module doc for rationale).
const EXCLUDED_PREFIXES: &[&str] = &["docs/superpowers/"];

/// Exact file paths excluded from the scan (see module doc for rationale).
const EXCLUDED_FILES: &[&str] = &["docs/RELEASING.md", "docs/maximum-feature-plan.md"];

/// `CARGO_MANIFEST_DIR` is `<root>/crates/codelore-lib`; two levels up is
/// the workspace root. Embedded at compile time, so it resolves under CI
/// too.
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root two levels above crates/codelore-lib")
        .to_path_buf()
}

fn collect_md_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return; // a missing root is fine — just nothing to scan
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_md_files(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
            out.push(path);
        }
    }
}

fn is_excluded(rel: &Path) -> bool {
    let rel_str = rel.to_string_lossy().replace('\\', "/");
    EXCLUDED_PREFIXES.iter().any(|p| rel_str.starts_with(p))
        || EXCLUDED_FILES.iter().any(|f| rel_str == *f)
}

/// If `line` contains a hard-coded "<digits> analyses" count that
/// disagrees with `real_count`, returns a description for the violation
/// report. Matches a run of ASCII digits, optional spaces, then the word
/// "analyses" (case-insensitive) at a word boundary — this catches both
/// plain prose ("the 54 analyses") and Markdown emphasis
/// ("**54 analyses**"), since the `**` markers sit outside the matched
/// span.
fn stale_count_in_line(line: &str, real_count: usize) -> Option<String> {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if !bytes[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        let digits = &line[start..i];
        let mut j = i;
        while j < bytes.len() && bytes[j] == b' ' {
            j += 1;
        }
        let rest = &line[j..];
        const WORD: &str = "analyses";
        // `.get()`, not byte-index slicing: `rest` may put the WORD.len()
        // cut point inside a multi-byte UTF-8 character (docs use math
        // symbols like 'σ'), which would panic on a raw `&rest[..N]`.
        if rest
            .get(..WORD.len())
            .is_some_and(|candidate| candidate.eq_ignore_ascii_case(WORD))
        {
            let boundary_ok = rest
                .as_bytes()
                .get(WORD.len())
                .is_none_or(|b| !b.is_ascii_alphanumeric());
            if boundary_ok {
                if let Ok(n) = digits.parse::<usize>() {
                    if n != real_count {
                        return Some(format!(
                            "\"{digits} analyses\" (registry currently has {real_count})"
                        ));
                    }
                }
            }
        }
    }
    None
}

#[test]
fn no_stale_analysis_count_in_docs() {
    let real_count = AnalysisName::all().len();
    let root = workspace_root();
    let mut files = Vec::new();
    collect_md_files(&root.join("docs"), &mut files);
    assert!(
        !files.is_empty(),
        "scanned zero docs/*.md files — doc-path resolution is broken"
    );

    let mut violations = Vec::new();
    for file in &files {
        let rel = file.strip_prefix(&root).unwrap_or(file);
        if is_excluded(rel) {
            continue;
        }
        let text = std::fs::read_to_string(file).expect("read doc file");
        for (line_idx, line) in text.lines().enumerate() {
            if let Some(found) = stale_count_in_line(line, real_count) {
                violations.push(format!("{}:{}: {found}", rel.display(), line_idx + 1));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "found {} stale hard-coded analysis count(s) in docs. Describe the registry by \
         shape instead (e.g. \"the full analysis registry, enumerated by \
         `AnalysisName::all()`\") so the doc can't drift again:\n{}",
        violations.len(),
        violations.join("\n"),
    );
}
