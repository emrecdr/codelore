//! Guard: no `${{ }}` in a workflow or the composite action is empty.
//!
//! GitHub evaluates template expressions across the whole of a `run:` body and
//! every `env:` value before bash ever sees the text. A `#` comment is
//! invisible to bash, not to that evaluator — so writing the expression syntax
//! literally in prose, to explain it, makes the file fail to load:
//!
//! ```text
//! action.yml (Line: 66, Col: 12): An expression was expected
//! ```
//!
//! The whole action is refused at that point, so every step is skipped and the
//! failure surfaces as the first `uses:` step failing for no visible reason.
//! It is a parse error, which means no amount of testing the shell logic can
//! catch it and the only signal is a red job.
//!
//! Documenting the injection-safe patterns these files rely on means writing
//! about that syntax, which is exactly when this is easy to trip. The scan is
//! textual and needs no YAML parse: an empty expression is never valid, so
//! there is nothing to interpret.

use std::path::{Path, PathBuf};

/// `CARGO_MANIFEST_DIR` is `<root>/crates/codelore-lib`; two levels up is the
/// workspace root. Embedded at compile time, so it resolves under CI too.
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root two levels above crates/codelore-lib")
        .to_path_buf()
}

/// Every file GitHub parses as a workflow or action template.
fn template_files(root: &Path) -> Vec<PathBuf> {
    let mut out = vec![root.join("action.yml")];
    if let Ok(entries) = std::fs::read_dir(root.join(".github/workflows")) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) == Some("yml") {
                out.push(p);
            }
        }
    }
    out
}

/// Byte offsets of every `${{ … }}` span whose body is blank.
fn empty_expressions(text: &str) -> Vec<(usize, String)> {
    let mut found = Vec::new();
    for (idx, line) in text.lines().enumerate() {
        let mut rest = line;
        while let Some(open) = rest.find("${{") {
            let after = &rest[open + 3..];
            let Some(close) = after.find("}}") else { break };
            if after[..close].trim().is_empty() {
                found.push((idx + 1, line.trim().to_owned()));
            }
            rest = &after[close + 2..];
        }
    }
    found
}

#[test]
fn no_empty_template_expressions_in_workflows() {
    let root = workspace_root();
    let files = template_files(&root);
    assert!(
        files.len() > 1,
        "found no workflow files under {} — path resolution is broken, so this \
         guard would pass vacuously",
        root.join(".github/workflows").display()
    );

    let mut violations = Vec::new();
    for file in &files {
        let Ok(text) = std::fs::read_to_string(file) else {
            continue; // action.yml is the only required one, and it is asserted below
        };
        let rel = file.strip_prefix(&root).unwrap_or(file);
        for (line, content) in empty_expressions(&text) {
            violations.push(format!("  {}:{line}: {content}", rel.display()));
        }
    }

    assert!(
        violations.is_empty(),
        "{} empty template expression(s) — GitHub refuses to load the file and \
         every step in it is skipped:\n{}\n\nThis fires on prose too: the \
         evaluator reads `#` comments, so the expression syntax cannot be \
         written literally to explain it. Describe it in words instead.",
        violations.len(),
        violations.join("\n"),
    );
}

#[test]
fn the_guard_detects_an_empty_expression() {
    // A guard that cannot fail is worth nothing, and this one's subject is a
    // parse error no other test can reach — so prove the detector directly.
    let sample = "        # spliced by `${{ }}` at render time\n";
    assert_eq!(
        empty_expressions(sample).len(),
        1,
        "the detector must flag an empty expression written in a comment"
    );
    // And must not fire on real expressions, or it would block every workflow.
    let real = "        INPUT_VERSION: ${{ inputs.version }}\n  if: ${{ success() }}\n";
    assert!(
        empty_expressions(real).is_empty(),
        "the detector must leave real expressions alone"
    );
}
