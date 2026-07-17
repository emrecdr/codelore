//! VCS-reading abstraction. The default impl is `gix`; a `GitCliRepo`
//! differential-test oracle cross-checks it.

use crate::{CommitEvent, FileChange, Hunk, Options, Result};

pub mod types;
pub use types::TagInfo;

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
    /// only, so an untracked file cannot affect them.
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

pub mod gix_repo;
pub use gix_repo::GixRepo;

pub mod git_cli_repo;
pub use git_cli_repo::GitCliRepo;
