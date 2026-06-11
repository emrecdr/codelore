//! VCS-reading abstraction. The default impl is `gix` in Plan 1;
//! a `GitCliRepo` differential-test oracle lands in Plan 6.

pub mod types;

pub use types::CommitMetadata;

use crate::{CommitEvent, FileChange, Hunk, Options, Result};

/// Read-only git operations needed by the codelore pipeline.
/// See spec §3.3.
pub trait Repo: Send + Sync {
    /// Walk commits matching `opts.after`/`opts.before`/`opts.commit_range`.
    /// Returns an iterator (Plan 4 will introduce Stream over async).
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

    /// Commit metadata not in `CommitEvent` (signed-by, signoffs).
    fn commit_metadata(&self, rev: &str) -> Result<CommitMetadata>;

    /// Return the full SHA-1 hex string of HEAD.
    /// Used by the persistent cache (Plan 8 §3) to build the cache key.
    fn head_sha(&self) -> Result<String>;

    /// Whether the working tree carries uncommitted modifications, untracked
    /// Tier-1 source files, or staged-but-uncommitted changes.
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

    /// Read the blob bytes at HEAD for `path` (POSIX-separated, repo-
    /// relative). Returns `Ok(None)` if the path isn't tracked at
    /// HEAD (deleted upstream, or a directory). Returns `Err` only
    /// on real I/O failure.
    ///
    /// HEAD-time scans (complexity, clones) use this instead of
    /// reading the working-tree file via `std::fs::read`. Reading
    /// blobs avoids three failure modes:
    ///   1. Bare repos have no working tree — `fs::read` always
    ///      fails. Blob reads work.
    ///   2. Dirty working trees: `fs::read` returns the user's
    ///      uncommitted edits, contaminating HEAD-time metrics with
    ///      changes that aren't yet in the fact store's commit
    ///      history.
    ///   3. Untracked files (created locally, never committed)
    ///      would be picked up by a working-tree walk but have no
    ///      blob — the HEAD-time semantic is to skip them, which
    ///      blob reads enforce by construction.
    ///
    /// Default impl returns `Ok(None)` so backends without an
    /// efficient blob lookup can opt out and fall back to the
    /// working-tree disk path.
    ///
    /// # Errors
    ///
    /// Returns an error on object-database I/O failures (corrupted
    /// pack, missing shallow object, etc.) but NOT on "path doesn't
    /// exist at HEAD" — that case returns `Ok(None)`.
    fn read_blob_at_head(&self, _path: &str) -> Result<Option<Vec<u8>>> {
        Ok(None)
    }
}

pub mod gix_repo;
pub use gix_repo::GixRepo;

pub mod git_cli_repo;
pub use git_cli_repo::GitCliRepo;
