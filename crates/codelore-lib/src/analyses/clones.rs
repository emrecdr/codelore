//! Clones analysis. Walks the working tree at HEAD, fingerprints
//! every function in every Tier-1 file, groups by structural digest, emits
//! one row per clone-family member.
//!
//! This analysis is HEAD-only: clones are computed against the working tree
//! the user has checked out. Historical clone tracking (when did this clone
//! family appear?) is a v1.x follow-up.
//!
//! Research basis: see `docs/research-foundations.md` entry "clones"
//! (Koschke, Falke & Frenzel, WCRE 2006 — AST suffix-tree clone
//! detection; Sajnani et al., ICSE 2016 — `SourcererCC`, the
//! index-then-probe pattern `CodeLore` follows).

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use walkdir::WalkDir;

use crate::analyses::query::query_map_collect;
use crate::clones::{CloneLanguage, extract_functions, group_clones};
use crate::facts::FactsDb;
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
#[tracing::instrument(name = "clones", skip_all, fields(min_revs = opts.min_revs))]
pub fn run_clones(opts: &Options) -> Result<Vec<ClonesRow>> {
    use rayon::iter::{IntoParallelIterator, ParallelIterator};
    // Combine --exclude patterns with any .codeloreignore
    // file at the repo root into one GlobSet, then short-circuit per-file walks
    // that match. The .git/target/node_modules hard-skips are kept as defaults.
    // Combined filter: --exclude globs + .gitignore + .git/info/exclude
    // + .codeloreignore. See `paths_filter::PathsFilter` for the
    // precedence rules.
    let filter = crate::paths_filter::PathsFilter::from_opts(opts)?;

    // Split into two phases — (1) a serial WalkDir+filter pass to
    // gather the candidate file list, (2) a parallel rayon pass to read +
    // tree-sitter-fingerprint each candidate. Mirrors the proven pattern
    // already used in `ingest::populate_clones_at_head` (parallelised in
    // Tier 2 quality work). The serial walk is cheap (filesystem traversal
    // + globset matching); the parallel phase is what scales with cores
    // for the tree-sitter parsing dominator.
    let candidates: Vec<(PathBuf, String, CloneLanguage)> = WalkDir::new(&opts.repo_path)
        .into_iter()
        .filter_map(std::result::Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter_map(|entry| {
            let path = entry.path();
            let lang = CloneLanguage::from_path(path)?;
            let rel = relative(&opts.repo_path, path);
            let rel_path = std::path::Path::new(&rel);
            // Always-on: never analyse .git/ contents (CodeLore's own
            // metadata isn't a valid source).
            if crate::paths_filter::is_git_metadata(rel_path) {
                return None;
            }
            // Combined filter: respects user --exclude, .codeloreignore,
            // and (by default) the project's .gitignore. Matching is
            // on the REPO-RELATIVE path, not the
            // absolute path.
            if filter.is_excluded(rel_path, false) {
                return None;
            }
            Some((path.to_path_buf(), rel, lang))
        })
        .collect();

    // Parallel phase: read + tree-sitter for each candidate.
    // `collect::<Result<Vec<_>>>()` preserves fail-fast semantics on
    // any extract_functions error.
    let all_fns: Vec<_> = candidates
        .into_par_iter()
        .filter_map(|(path, rel, lang)| -> Option<Result<Vec<_>>> {
            let code = fs::read(&path).ok()?;
            // Skip oversized files (generated / minified) before
            // tree-sitter to avoid OOM / stack-overflow on deeply nested
            // generated code.
            if code.len() > crate::constants::DEFAULT_MAX_AST_FILE_BYTES {
                tracing::debug!(
                    "clones: skipping {rel} ({size} bytes > {cap}-byte AST cap)",
                    size = code.len(),
                    cap = crate::constants::DEFAULT_MAX_AST_FILE_BYTES,
                );
                return None;
            }
            Some(
                extract_functions(&rel, &code, lang)
                    .map_err(|e| CodeLoreError::Analysis(format!("clones: extract {rel}: {e}"))),
            )
        })
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .flatten()
        .collect();
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

/// Memoised [`run_clones`] for the one `FactsDb` that scores a repo more than
/// once in a single process.
///
/// `run_clones` itself stays db-free so the standalone `clones` analysis and
/// `diff`'s at-a-rev walk (neither of which owns the scoring `FactsDb`) keep
/// calling it directly. The only in-process caller that scores the same tree
/// twice — the agent-loop gate's projected-health engine, which runs
/// code-health once for the HEAD baseline and again for the substituted
/// projection — routes through here so the identical second working-tree walk
/// is served from `db`'s single-slot memo instead of re-fingerprinting every
/// Tier-1 function. Returns a shared handle; callers read it by reference.
///
/// # Errors
///
/// Propagates any [`run_clones`] error on a memo miss.
pub(crate) fn run_clones_memoised(
    db: &FactsDb,
    opts: &Options,
) -> Result<std::rc::Rc<Vec<ClonesRow>>> {
    let memo = db.analysis_memo::<crate::analyses::memo::ClonesMemo>();
    if let Some(cached) = memo.get() {
        return Ok(cached);
    }
    let rows = std::rc::Rc::new(run_clones(opts)?);
    memo.put(rows.clone());
    Ok(rows)
}

/// HEAD-faithful per-file clone-family membership counts, read from the
/// `clones` table populated at ingest from HEAD blobs
/// (`facts::ingest::populate_clones_at_head`). One count per path: the number
/// of that file's functions that belong to some clone family — the same
/// per-path tally [`run_clones`] yields for a clean working tree, so the gate
/// baseline (this) and the gate projection (the working-tree walk) agree
/// exactly when the tree equals HEAD.
///
/// # Errors
///
/// Returns [`CodeLoreError::Analysis`] on a fact-store / SQL error.
pub(crate) fn head_clone_counts(db: &FactsDb) -> Result<HashMap<String, u32>> {
    let rows = query_map_collect(
        db,
        "SELECT path, COUNT(*) FROM clones GROUP BY path",
        [],
        "head clone counts",
        |r| {
            let path = r.get::<_, String>(0)?;
            let count = u32::try_from(r.get::<_, i64>(1)?).unwrap_or(u32::MAX);
            Ok((path, count))
        },
    )?;
    Ok(rows.into_iter().collect())
}

fn relative(root: &Path, abs: &Path) -> String {
    // Normalise to POSIX `/` so this matches `changes.path` (git always
    // emits `/`). See `crate::paths::to_posix` for the rationale.
    abs.strip_prefix(root)
        .map_or_else(|_| crate::paths::to_posix(abs), crate::paths::to_posix)
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

    #[test]
    fn run_clones_memoised_serves_second_call_from_the_memo() {
        // Two `run_clones_memoised` calls on one FactsDb must return the SAME
        // allocation: the first walks + fingerprints, the second is served
        // whole from the single-slot memo (Rc::ptr_eq proves no recompute).
        let fx = crate::test_support::differential_repo::build();
        let repo = crate::repo::GixRepo::open(fx.dir.path()).expect("open");
        let db = crate::facts::FactsDb::new_in_memory().expect("db");
        let opts = Options {
            repo_path: fx.dir.path().to_path_buf(),
            min_clone_node_count: 0,
            ..Options::default()
        };
        db.ingest(&repo, &opts).expect("ingest");

        assert!(
            db.analysis_memo::<crate::analyses::memo::ClonesMemo>()
                .get()
                .is_none(),
            "memo starts empty"
        );
        let first = run_clones_memoised(&db, &opts).expect("first walk");
        assert!(
            db.analysis_memo::<crate::analyses::memo::ClonesMemo>()
                .get()
                .is_some(),
            "first call must populate the memo",
        );
        let second = run_clones_memoised(&db, &opts).expect("second walk");
        assert!(
            std::rc::Rc::ptr_eq(&first, &second),
            "second call must be served from the memo (same Rc allocation)",
        );
    }
}
