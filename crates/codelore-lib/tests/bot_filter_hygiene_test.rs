//! Guard: no canonical-level `BOOL_OR(is_bot)` / `HAVING NOT BOOL_OR` bot
//! filtering outside `analyses/query.rs`.
//!
//! Bot filtering is routed through the shared `HUMAN_ALIASES_CTE` in
//! `analyses/query.rs`, which resolves humanity per alias rather than
//! collapsing it to a single `BOOL_OR` per canonical identity — a canonical
//! with a mix of human and bot aliases is silently misclassified by the old
//! per-canonical collapse. Nothing mechanical stops a future query from
//! reintroducing that pattern, so this test fails the gate if the literal
//! reappears anywhere under `analyses/` except `query.rs` itself, which
//! legitimately names the pattern in `HUMAN_ALIASES_CTE`'s explanatory doc
//! comment.

use std::path::{Path, PathBuf};

/// Root scanned, relative to the workspace root.
const SCANNED: &str = "crates/codelore-lib/src/analyses";

/// File exempt from the ban: it names the retired pattern in an explanatory
/// doc comment describing what `HUMAN_ALIASES_CTE` replaces.
const EXEMPT_FILE: &str = "query.rs";

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

#[test]
fn no_canonical_level_bool_or_is_bot_outside_query_rs() {
    let root = workspace_root();
    let mut files = Vec::new();
    collect_rs_files(&root.join(SCANNED), &mut files);
    assert!(
        !files.is_empty(),
        "scanned zero .rs files — source-path resolution is broken"
    );

    let mut violations = Vec::new();
    for file in &files {
        if file.file_name().and_then(|n| n.to_str()) == Some(EXEMPT_FILE) {
            continue;
        }
        let text = std::fs::read_to_string(file).expect("read source file");
        for (line_idx, line) in text.lines().enumerate() {
            if line.contains("BOOL_OR(is_bot)") || line.contains("HAVING NOT BOOL_OR") {
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
        "found {} canonical-level BOOL_OR(is_bot) bot-filter collapse(s) outside query.rs. \
         Route bot filtering through `HUMAN_ALIASES_CTE` (analyses/query.rs) instead — it \
         resolves humanity per alias, not per canonical identity:\n{}",
        violations.len(),
        violations.join("\n"),
    );
}
