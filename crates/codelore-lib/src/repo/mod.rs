//! VCS-reading abstraction. The default impl is `gix`; a `GitCliRepo`
//! differential-test oracle cross-checks it.

use crate::{CommitEvent, FileChange, Hunk, Options, Result};

pub mod types;
pub use types::{TagInfo, WorktreeChange, WorktreeChangeKind};

/// Read-only git operations needed by the codelore pipeline.
/// See spec §3.3.
pub trait Repo: Send + Sync {
    /// Walk commits matching `opts.after`/`opts.before`.
    /// Returns an iterator over the resulting commit events.
    fn walk_commits<'a>(
        &'a self,
        opts: &'a Options,
    ) -> Result<Box<dyn Iterator<Item = Result<CommitEvent>> + Send + 'a>>;

    /// Per-file changes for one commit.
    fn changed_files(&self, rev: &str) -> Result<Vec<FileChange>>;

    /// Hunks within one (commit, path) pair.
    fn diff_hunks(&self, rev: &str, path: &str) -> Result<Vec<Hunk>>;

    /// `.mailmap`-aware author identity canonicalization. Returns the canonical
    /// email for the given (name, email) pair after applying any matching
    /// `.mailmap` rule.
    ///
    /// `name` and `email` are BOTH significant — `.mailmap` supports two
    /// rule formats:
    ///   - `Canonical Name <canonical@email> <old@email>`        (email-only match)
    ///   - `Canonical Name <canonical@email> Old Name <old@email>` (name+email match)
    ///
    /// Email-only matches succeed even with `name = ""`, but name+email
    /// matches REQUIRE the caller to pass the actual author name. Earlier
    /// versions of this trait passed only `email`; the differential test
    /// fixtures didn't include name+email rules so the bug was invisible —
    /// real repos with `.mailmap` files using the name+email form had
    /// `GitCliRepo` and `GixRepo` produce different canonical authors for
    /// the same commit (`GixRepo::walk_commits` has its own inline
    /// resolution that already passes name+email, while `GitCliRepo::walk_commits`
    /// went through this trait method).
    fn resolve_alias(&self, name: &str, email: &str) -> String;

    /// Return the full SHA-1 hex string of HEAD.
    /// Used by the persistent cache to build the cache key.
    fn head_sha(&self) -> Result<String>;

    /// Whether tracked content differs from `HEAD` — staged changes (index
    /// vs. `HEAD`) or unstaged changes (worktree vs. index). Untracked
    /// files are excluded: every caller (the `calibrate-defects` mining
    /// guard, the cache-hit staleness warning, the dirty cache-write skip)
    /// protects HEAD-time metrics computed over `tracked_paths_at_head()`
    /// only. Exception: a submodule whose only change is untracked content
    /// in its own worktree may report dirty (backend-dependent).
    ///
    /// Used by the persistent-cache code path to emit a `tracing::warn!`
    /// when a cache HIT occurs on a dirty tree — HEAD-time metrics
    /// (`complexity`, `clones`) are computed from the working tree at
    /// ingest time, so a cached result keyed off `head_sha` can mismatch
    /// what the user sees on disk now. The warning recommends `--no-cache`.
    ///
    /// Default impl returns `false` (assume clean) so backends without a
    /// cheap dirty-check can opt out. Implementations that fail to detect
    /// MUST return `false` rather than propagating an error — a missed
    /// warning is better than a hard analyze failure on a state-detection
    /// edge case (e.g. unusual submodule layout).
    fn is_worktree_dirty(&self) -> bool {
        false
    }

    /// Whether the repository is partway through a merge, rebase,
    /// cherry-pick, or revert — an ambiguous-HEAD state where the working
    /// tree and `HEAD` no longer describe one coherent commit. True when any
    /// of `MERGE_HEAD`, `CHERRY_PICK_HEAD`, `REVERT_HEAD`, `rebase-merge/`,
    /// or `rebase-apply/` is present in the repository's git dir (worktree-
    /// correct: a linked worktree keeps this state in its own git dir, not
    /// the common one).
    ///
    /// The agent-loop briefing tools call this so they can disclose the
    /// ambiguous state honestly rather than presenting committed-`HEAD`
    /// history as the whole picture.
    ///
    /// Default impl returns `false` so backends without a cheap check can opt
    /// out. Like [`is_worktree_dirty`](Self::is_worktree_dirty), detection is
    /// a hint rather than a contract: an implementation that cannot determine
    /// the state returns `false` (a missed note) instead of surfacing an
    /// error.
    fn merge_or_rebase_in_progress(&self) -> bool {
        false
    }

    /// Read the blob bytes at revision `rev` for `path` (POSIX-
    /// separated, repo-relative). `rev` is any git revision the backend
    /// can resolve — a commit SHA, `"HEAD"`, a tag, etc. Returns
    /// `Ok(None)` if the path isn't a tracked blob at that revision
    /// (deleted there, a directory, or a submodule gitlink). Returns
    /// `Err` only on real object-database I/O failure (corrupted pack,
    /// missing shallow object) — NOT on "path doesn't exist at `rev`".
    ///
    /// Reading blobs from the object database (rather than the working
    /// tree via `std::fs::read`) is what lets HEAD-time scans
    /// (complexity, clones) AND historical scans (architecture-trend)
    /// work on bare repos, ignore dirty-worktree edits, and skip
    /// untracked files by construction.
    ///
    /// Default impl returns `Ok(None)` so backends without an efficient
    /// blob lookup can opt out and fall back to the working-tree path.
    fn read_blob_at(&self, _rev: &str, _path: &str) -> Result<Option<Vec<u8>>> {
        Ok(None)
    }

    /// Read the blob bytes at HEAD for `path`. Convenience wrapper over
    /// [`read_blob_at`](Self::read_blob_at) — the HEAD-time scans' entry
    /// point. Backends override `read_blob_at`, not this.
    ///
    /// # Errors
    ///
    /// Propagates object-database I/O failures from `read_blob_at`;
    /// "not tracked at HEAD" is `Ok(None)`, not an error.
    fn read_blob_at_head(&self, path: &str) -> Result<Option<Vec<u8>>> {
        self.read_blob_at("HEAD", path)
    }

    /// Enumerate tracked working-tree changes vs HEAD (union of staged and
    /// unstaged, net-classified; untracked files excluded; symlinks and
    /// submodule pointers excluded; sorted by path). Errors on unmerged
    /// (conflict) entries. Hint quality: backends agree via differential
    /// tests.
    ///
    /// Default impl returns an empty list so backends without a status
    /// facility can opt out — the same convention as
    /// [`is_worktree_dirty`](Self::is_worktree_dirty).
    fn worktree_changes(&self) -> Result<Vec<WorktreeChange>> {
        Ok(Vec::new())
    }

    /// Every regular-file blob path (the `0o100xxx` mode class — canonical
    /// `100644`/`100755` plus legacy non-canonical variants like `100664`)
    /// in the HEAD commit's tree, repo-relative with `/` separators, sorted
    /// ascending. Symlinks (`120000`) and submodule gitlinks (`160000`) are
    /// excluded — neither carries source bytes the HEAD-time scans can parse.
    ///
    /// Unlike the walk-derived live-path reconstruction (most recent
    /// change per path is not a deletion), this reads the tree directly,
    /// so it works without any commit history in the fact store — the
    /// head-only ingest mode depends on that.
    fn tracked_paths_at_head(&self) -> Result<Vec<String>>;

    /// Return all git tags in this repository, sorted ascending by
    /// `(date, name)`.
    ///
    /// Date semantics:
    /// - **Annotated tags** — the tagger timestamp (when `git tag -a` was run).
    /// - **Lightweight tags** — the target commit's committer timestamp.
    ///
    /// `target_rev` is always the peeled commit SHA (40-char hex); for
    /// annotated tags this is the commit the tag object ultimately points at,
    /// not the tag object's own OID.
    fn tags(&self) -> Result<Vec<TagInfo>>;
}

/// Merge a `worktree_changes` candidate into the per-path map. A reported
/// rename source (`Some`) wins over `None` regardless of arrival order —
/// the same path can reach the map from both status streams (e.g. a staged
/// rename destination that was then edited again in the worktree).
fn add_worktree_candidate(
    candidates: &mut std::collections::BTreeMap<String, Option<String>>,
    path: String,
    rename_from: Option<String>,
) {
    let slot = candidates.entry(path).or_insert(None);
    if slot.is_none() {
        *slot = rename_from;
    }
}

/// Net-classify merged `worktree_changes` candidates. Shared by both
/// backends so the classification table cannot drift between them:
///
/// | blob at HEAD | file on disk | result   |
/// |--------------|--------------|----------|
/// | no           | yes          | Added    |
/// | yes          | no           | Deleted  |
/// | yes          | yes          | Modified |
/// | no           | no           | dropped  |
///
/// The dropped row is the staged-add-then-worktree-delete case (`AD` in
/// porcelain terms): the path nets out identical to HEAD, so it is not a
/// change. The map is keyed by path, so the output is already sorted
/// ascending and free of duplicates.
fn net_classify_candidates<R: Repo>(
    repo: &R,
    worktree_root: &std::path::Path,
    candidates: std::collections::BTreeMap<String, Option<String>>,
) -> Result<Vec<WorktreeChange>> {
    let mut changes = Vec::with_capacity(candidates.len());
    for (path, rename_from) in candidates {
        let at_head = repo.read_blob_at_head(&path)?.is_some();
        let on_disk = std::fs::metadata(worktree_root.join(&path)).is_ok_and(|m| m.is_file());
        let kind = match (at_head, on_disk) {
            (false, true) => WorktreeChangeKind::Added,
            (true, false) => WorktreeChangeKind::Deleted,
            (true, true) => WorktreeChangeKind::Modified,
            (false, false) => continue,
        };
        changes.push(WorktreeChange {
            path,
            kind,
            rename_from,
        });
    }
    Ok(changes)
}

pub mod gix_repo;
pub use gix_repo::GixRepo;

pub mod git_cli_repo;
pub use git_cli_repo::GitCliRepo;
