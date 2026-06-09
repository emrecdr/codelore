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

    /// .mailmap-aware author email canonicalization.
    fn resolve_alias(&self, email: &str) -> String;

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
}

pub mod gix_repo;
pub use gix_repo::GixRepo;

pub mod git_cli_repo;
pub use git_cli_repo::GitCliRepo;
