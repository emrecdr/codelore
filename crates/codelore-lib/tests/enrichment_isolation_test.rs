//! Scoring-isolation guard (Contract 2): no module in the scoring path imports
//! the advisory `enrichment` layer. The dependency arrow points one way —
//! enrichment reads the analyses, never the reverse — so enrichment can never
//! perturb an analysis row, a gate verdict, an exit code, or a fact-store cache
//! key. Enforced structurally by scanning the source of every scoring module at
//! test time for a reference to `crate::enrichment`.

use std::fs;
use std::path::{Path, PathBuf};

/// The scoring-path roots, relative to this crate's `src/`. Directories are
/// scanned recursively; `calibration.rs` is a single file.
const SCORING_ROOTS: &[&str] = &[
    "analyses",
    "quality_gates",
    "facts",
    "calibration.rs",
    "defect_calibration",
    "provenance",
    "output",
];

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

#[test]
fn scoring_modules_never_import_enrichment() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");

    let mut files = Vec::new();
    for root in SCORING_ROOTS {
        let path = src.join(root);
        assert!(
            path.exists(),
            "scoring root {} does not exist — update SCORING_ROOTS",
            path.display()
        );
        collect_rs_files(&path, &mut files);
    }
    assert!(
        !files.is_empty(),
        "expected to scan some scoring-path source files"
    );

    let mut offenders = Vec::new();
    for file in &files {
        let text = fs::read_to_string(file).expect("read source file");
        if text.contains("use crate::enrichment") || text.contains("crate::enrichment::") {
            offenders.push(file.display().to_string());
        }
    }

    assert!(
        offenders.is_empty(),
        "scoring modules must not import crate::enrichment (advisory-only): {offenders:?}"
    );
}
