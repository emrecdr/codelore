//! Combined exclusion filter for HEAD-time path walks.
//!
//! Sources, applied in order of precedence (later wins):
//! 1. Hardcoded essentials: just `.git/` (codelore can't analyze its
//!    own metadata).
//! 2. `.gitignore`, `.git/info/exclude`, parent-directory `.gitignore`s
//!    via the `ignore` crate's `gitignore::GitignoreBuilder`. Respects
//!    standard gitignore syntax including negation (`!keep.txt`),
//!    directory-only patterns (`build/`), and per-directory inheritance.
//! 3. `.codeloreignore` at the repo root — `CodeLore`-specific additions
//!    that the user wouldn't put in `.gitignore` (e.g. translation
//!    files that ARE checked in but you don't want `CodeLore` to analyze).
//! 4. User `--exclude '<glob>'` CLI flags. Treated as plain globset
//!    patterns (no gitignore semantics) since they're typed at the
//!    shell.
//!
//! Why .gitignore by default: it's the user's project-local "what's
//! not my code" manifest — exactly the answer to "should `CodeLore`
//! analyze this?" The result is that out of the box, `CodeLore` skips
//! `node_modules`, `target`, `dist`, lockfiles, etc. for any
//! conventionally-set-up project, without `CodeLore` reinventing the
//! list. Users with unconventional layouts can override via
//! `.codeloreignore` or `--include-ignored` (opt-out).

use std::path::Path;

use crate::{CodeLoreError, Options, Result};

/// Combined matcher. `is_excluded(rel_path)` returns true when the
/// path should be skipped by any HEAD-time scan (complexity, clones,
/// etc.).
pub struct PathsFilter {
    /// User `--exclude '<glob>'` patterns. Matched as plain globset.
    globset: globset::GlobSet,
    /// .gitignore + .codeloreignore + .git/info/exclude rules.
    gitignore: ignore::gitignore::Gitignore,
}

impl PathsFilter {
    /// Build the combined filter for `opts.repo_path`. Honours
    /// `opts.exclude_patterns` (user `--exclude`) and the auto-detected
    /// gitignore set unless `opts.include_ignored` is true.
    pub fn from_opts(opts: &Options) -> Result<Self> {
        let mut gb = ignore::gitignore::GitignoreBuilder::new(&opts.repo_path);
        if !opts.include_ignored {
            // .gitignore at the repo root + auto-discovery of parent
            // .gitignores + .git/info/exclude. add() returns None on
            // success or Some(err) on parse error; we surface parse
            // errors but tolerate missing files.
            let gitignore_path = opts.repo_path.join(".gitignore");
            if gitignore_path.is_file()
                && let Some(err) = gb.add(&gitignore_path)
            {
                return Err(CodeLoreError::Analysis(format!(
                    ".gitignore at {}: {err}",
                    gitignore_path.display()
                )));
            }
            // .git/info/exclude is the user's local-only ignore list
            // (not checked in). Honour it too — same precedence as
            // .gitignore per the spec.
            let info_exclude = opts.repo_path.join(".git/info/exclude");
            if info_exclude.is_file()
                && let Some(err) = gb.add(&info_exclude)
            {
                return Err(CodeLoreError::Analysis(format!(
                    ".git/info/exclude at {}: {err}",
                    info_exclude.display()
                )));
            }
            // .codeloreignore — `CodeLore`-specific additions. Lives
            // alongside .gitignore in the user's repo.
            let codelore_ignore = opts.repo_path.join(".codeloreignore");
            if codelore_ignore.is_file()
                && let Some(err) = gb.add(&codelore_ignore)
            {
                return Err(CodeLoreError::Analysis(format!(".codeloreignore: {err}")));
            }
        }
        let gitignore = gb
            .build()
            .map_err(|e| CodeLoreError::Analysis(format!("build gitignore matcher: {e}")))?;

        let mut gsb = globset::GlobSetBuilder::new();
        for pat in &opts.exclude_patterns {
            let g = globset::Glob::new(pat)
                .map_err(|e| CodeLoreError::Analysis(format!("--exclude {pat:?}: {e}")))?;
            gsb.add(g);
        }
        let globset = gsb
            .build()
            .map_err(|e| CodeLoreError::Analysis(format!("build --exclude globset: {e}")))?;

        Ok(Self { globset, gitignore })
    }

    /// True if `rel_path` (relative to the repo root) should be skipped.
    /// Pass `is_dir` so directory-only `.gitignore` rules (`build/`)
    /// match correctly when called per-file.
    #[must_use]
    pub fn is_excluded(&self, rel_path: &Path, is_dir: bool) -> bool {
        if self.globset.is_match(rel_path) {
            return true;
        }
        // matched_path_or_any_parents() also checks ancestor dirs so a
        // file under `dist/foo/bar.js` is excluded when only `dist/` is
        // listed. ignore_case=false since gitignore is case-sensitive
        // on Linux (the platform git was designed for).
        match self.gitignore.matched_path_or_any_parents(rel_path, is_dir) {
            ignore::Match::Ignore(_) => true,
            ignore::Match::Whitelist(_) | ignore::Match::None => false,
        }
    }
}

/// Always-on hardcoded skip — `.git/` directory contents. We can never
/// analyze our own metadata source.
#[must_use]
pub fn is_git_metadata(rel_path: &Path) -> bool {
    rel_path
        .components()
        .next()
        .and_then(|c| c.as_os_str().to_str())
        == Some(".git")
}
