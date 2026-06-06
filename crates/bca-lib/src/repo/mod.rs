//! VCS-reading abstraction. The default impl is `gix` in Plan 1;
//! a `GitCliRepo` differential-test oracle lands in Plan 6.

pub mod types;

pub use types::CommitMetadata;

use crate::{CommitEvent, FileChange, Hunk, Options, Result};

/// Read-only git operations needed by the bca pipeline.
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
}
