//! Scoring-isolation guard: no module in the scoring path references the
//! advisory `enrichment` layer. The dependency arrow points one way —
//! enrichment reads the analyses, never the reverse — so enrichment can never
//! perturb an analysis row, a gate verdict, an exit code, or a fact-store
//! cache key.
//!
//! The scan covers EVERY `.rs` file under `src/` except a named exclusion
//! list, so a new module is scored-by-default rather than unguarded until
//! someone remembers to list it. Comments are stripped before matching so
//! prose that merely mentions the layer (doc links, rationale notes) does not
//! trip the guard; the matcher then requires `enrichment` to appear as a path
//! segment (`enrichment::` or `::enrichment`), which a path-shaped string
//! literal like `"src/enrichment/fact_sheet.rs"` does not.

use std::fs;
use std::path::{Path, PathBuf};

/// Files allowed to reference `enrichment`, with the reason each is exempt:
/// the layer itself, the crate root that declares the module, and the CLI
/// facade that re-exports it for the advisory commands.
const EXEMPT: &[&str] = &["enrichment", "lib.rs", "cli_api.rs"];

/// Collect every `.rs` file at or beneath `path` (a file or a directory).
fn collect_rs_files(path: &Path, out: &mut Vec<PathBuf>) {
    if path.is_file() {
        if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path.to_path_buf());
        }
        return;
    }
    if path.is_dir() {
        for entry in fs::read_dir(path).expect("read scoring dir") {
            let entry = entry.expect("dir entry");
            collect_rs_files(&entry.path(), out);
        }
    }
}

/// Cut each line at its first `//`, removing line and doc comments. Coarse by
/// design: a `//` inside a string literal also truncates the rest of that
/// line, which can only make the guard miss eccentric single-line layouts,
/// never flag prose.
fn strip_line_comments(text: &str) -> String {
    text.lines()
        .map(|l| l.split("//").next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Does comment-stripped source reference `enrichment` as a PATH SEGMENT?
/// `enrichment::` catches absolute, relative, and grouped imports plus
/// expression paths; `::enrichment` catches bare imports (`use
/// crate::enrichment;`, `use super::enrichment as x;`). Prose mentions and
/// `/`-separated file-path strings match neither.
fn references_enrichment(text: &str) -> bool {
    let code = strip_line_comments(text);
    code.contains("enrichment::") || code.contains("::enrichment")
}

#[test]
fn scoring_modules_never_import_enrichment() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");

    let mut files = Vec::new();
    for entry in fs::read_dir(&src).expect("read src/") {
        let entry = entry.expect("dir entry");
        let name = entry.file_name();
        let name = name.to_str().expect("utf-8 file name");
        if EXEMPT.contains(&name) {
            continue;
        }
        collect_rs_files(&entry.path(), &mut files);
    }
    assert!(
        files.len() > 50,
        "scanned only {} files — the src/ walk is broken, not the codebase clean",
        files.len()
    );

    let mut offenders = Vec::new();
    for file in &files {
        let text = fs::read_to_string(file).expect("read source file");
        if references_enrichment(&text) {
            offenders.push(file.display().to_string());
        }
    }

    assert!(
        offenders.is_empty(),
        "scoring modules must not import crate::enrichment (advisory-only): {offenders:?}"
    );
}

/// Anti-vacuity: the guard's own matcher must flag every import form and
/// stay quiet on every prose form that exists in the tree today.
#[test]
fn the_enrichment_matcher_discriminates() {
    let violations = [
        "use crate::enrichment::fact_sheet::FactSheet;",
        "use crate::{analyses, enrichment::client};",
        "use super::super::enrichment;",
        "use crate::enrichment as advisory;",
        "let s = crate::enrichment::narrate(&row);",
    ];
    for v in violations {
        assert!(
            references_enrichment(v),
            "matcher missed an import form: {v}"
        );
    }

    let clean = [
        "//! Like [`crate::enrichment::fact_sheet`], it never computes anything new.",
        "// The improving/degrading split is an opt-in enrichment computed by",
        "let path = \"crates/codelore-lib/src/enrichment/fact_sheet.rs\".to_string();",
        "let enrichment_flag = true;",
        "/// Kamei 14-feature JIT-SDP canonical change vector enrichment.",
    ];
    for c in clean {
        assert!(
            !references_enrichment(c),
            "matcher false-positived on prose: {c}"
        );
    }
}
