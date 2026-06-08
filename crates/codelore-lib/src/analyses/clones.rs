//! Clones analysis (Plan 7). Walks the working tree at HEAD, fingerprints
//! every function in every Tier-1 file, groups by structural digest, emits
//! one row per clone-family member.
//!
//! This analysis is HEAD-only: clones are computed against the working tree
//! the user has checked out. Historical clone tracking (when did this clone
//! family appear?) is a v1.x follow-up.

use std::fs;
use std::path::Path;

use serde::Serialize;
use walkdir::WalkDir;

use crate::clones::{CloneLanguage, extract_functions, group_clones};
use crate::options::Options;
use crate::{CodeLoreError, Result};

/// One row in the clones analysis output. Members of the same clone family
/// share `clone_group_id` and `fingerprint`.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct ClonesRow {
    pub clone_group_id: u32,
    pub fingerprint: String,
    pub entity: String,
    pub function: String,
    pub start_line: u32,
    pub end_line: u32,
    pub node_count: u32,
    pub similarity: f64,
    pub family_size: u32,
}

/// Run the clones analysis at HEAD. Walks `opts.repo_path`'s working tree,
/// reads each Tier-1 file, fingerprints every function, groups, and returns
/// one `ClonesRow` per clone-family member.
pub fn run_clones(opts: &Options) -> Result<Vec<ClonesRow>> {
    // Plan 8 §2 Task 8: combine --exclude patterns with any .codeloreignore
    // file at the repo root into one GlobSet, then short-circuit per-file walks
    // that match. The .git/target/node_modules hard-skips are kept as defaults.
    let exclude_set = build_exclude_set(opts)?;

    let mut all_fns = Vec::new();
    for entry in WalkDir::new(&opts.repo_path)
        .into_iter()
        .filter_map(std::result::Result::ok)
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        // Default hard-skip: vendored / build-output directories that virtually
        // never contain user-meaningful clones.
        if path.components().any(|c| {
            matches!(
                c.as_os_str().to_str(),
                Some(".git" | "target" | "node_modules")
            )
        }) {
            continue;
        }
        let Some(lang) = CloneLanguage::from_path(path) else {
            continue;
        };
        let rel = relative(&opts.repo_path, path);
        // User-configured exclusions (--exclude + .codeloreignore).
        if exclude_set.is_match(&rel) {
            continue;
        }
        let Ok(code) = fs::read(path) else {
            continue; // unreadable file — silently skip
        };
        let fns = extract_functions(&rel, &code, lang)
            .map_err(|e| CodeLoreError::Analysis(format!("clones: extract {rel}: {e}")))?;
        all_fns.extend(fns);
    }
    let groups = group_clones(all_fns, opts.min_clone_node_count);

    let mut rows = Vec::new();
    for group in groups {
        let family_size = u32::try_from(group.members.len()).unwrap_or(u32::MAX);
        for member in &group.members {
            rows.push(ClonesRow {
                clone_group_id: group.clone_group_id,
                fingerprint: member.fingerprint.hex(),
                entity: member.path.clone(),
                function: member.function_name.clone(),
                start_line: member.start_line,
                end_line: member.end_line,
                node_count: member.fingerprint.node_count,
                similarity: 1.0, // T1+T2 = exact match
                family_size,
            });
        }
    }
    Ok(rows)
}

fn relative(root: &Path, abs: &Path) -> String {
    // Normalize to forward-slash so this path matches the `changes.path`
    // format (always `/`, from git). On Unix `MAIN_SEPARATOR` is `/` so
    // the replace is a no-op; on Windows it rewrites `\` to `/`.
    abs.strip_prefix(root).map_or_else(
        |_| {
            abs.to_string_lossy()
                .replace(std::path::MAIN_SEPARATOR, "/")
        },
        |p| p.to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/"),
    )
}

/// Build the combined exclude `GlobSet` from `opts.exclude_patterns` plus
/// any `.codeloreignore` file at the repo root. Lines starting with `#` and
/// blank lines are ignored, matching .gitignore convention.
fn build_exclude_set(opts: &Options) -> Result<globset::GlobSet> {
    let mut b = globset::GlobSetBuilder::new();
    for pat in &opts.exclude_patterns {
        let g = globset::Glob::new(pat)
            .map_err(|e| CodeLoreError::Analysis(format!("clones: --exclude {pat:?}: {e}")))?;
        b.add(g);
    }
    let ignore_path = opts.repo_path.join(".codeloreignore");
    if let Ok(contents) = std::fs::read_to_string(&ignore_path) {
        for line in contents.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let g = globset::Glob::new(line).map_err(|e| {
                CodeLoreError::Analysis(format!(".codeloreignore line {line:?}: {e}"))
            })?;
            b.add(g);
        }
    }
    b.build()
        .map_err(|e| CodeLoreError::Analysis(format!("clones: build globset: {e}")))
}

#[cfg(all(test, feature = "test-support"))]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn finds_type2_clone_pair_in_a_tempdir() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.rs");
        let b = dir.path().join("b.rs");
        let mut fa = std::fs::File::create(&a).unwrap();
        writeln!(
            fa,
            "fn add(a: i32, b: i32) -> i32 {{ let x = 1; let y = 2; a + b + x + y }}"
        )
        .unwrap();
        let mut fb = std::fs::File::create(&b).unwrap();
        writeln!(
            fb,
            "fn mul(p: u64, q: u64) -> u64 {{ let s = 9; let t = 7; p + q + s + t }}"
        )
        .unwrap();

        let opts = Options {
            repo_path: dir.path().to_path_buf(),
            min_clone_node_count: 0, // include this small fn
            ..Options::default()
        };
        let rows = run_clones(&opts).unwrap();
        assert_eq!(rows.len(), 2, "Type 2 pair → 2 rows in 1 family");
        assert_eq!(rows[0].clone_group_id, rows[1].clone_group_id);
        assert_eq!(rows[0].family_size, 2);
        let entities: Vec<_> = rows.iter().map(|r| r.entity.as_str()).collect();
        assert!(entities.iter().any(|e| e.ends_with("a.rs")));
        assert!(entities.iter().any(|e| e.ends_with("b.rs")));
    }
}
