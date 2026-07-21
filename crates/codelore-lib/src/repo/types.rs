//! Repo-layer value types shared across both `GixRepo` and `GitCliRepo`
//! backends and exposed through the `Repo` trait.

use time::OffsetDateTime;

/// One tracked path whose content differs from HEAD (staged, unstaged, or
/// both). `kind` is the NET classification vs HEAD; `rename_from` is set on
/// the destination entry when the backend reported a rename (the source
/// appears as its own `Deleted` entry).
///
/// Serde derives because the change-set report embeds these entries and
/// round-trips them through its JSON sidecar cache.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct WorktreeChange {
    /// Repo-relative, `/`-separated path as it exists in the working tree
    /// (for `Deleted` entries: as it existed at HEAD).
    pub path: String,
    /// Net classification of this path's content vs HEAD.
    pub kind: WorktreeChangeKind,
    /// The rename source path when the backend detected this entry as the
    /// destination of a rename; `None` otherwise.
    pub rename_from: Option<String>,
}

/// Net classification of a working-tree path vs HEAD. A path staged as
/// added then deleted from the worktree nets out to no change and is not
/// reported at all.
///
/// Serialises lowercase (`"added"` / `"modified"` / `"deleted"`) to match
/// the string form the change-set report's per-file rows use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorktreeChangeKind {
    /// Not a blob at HEAD; present in the working tree.
    Added,
    /// A blob at HEAD; present in the working tree with different content.
    Modified,
    /// A blob at HEAD; absent from the working tree.
    Deleted,
}

/// A git tag with its resolved target commit OID and the date used for sorting.
///
/// Date semantics follow the git convention for time-ordered tag listing:
/// - **Annotated tags** use the *tagger* timestamp (the date the tag object
///   was created, not when the commit was made).
/// - **Lightweight tags** use the target commit's *committer* timestamp.
///
/// The `Vec<TagInfo>` returned by [`super::Repo::tags`] is sorted ascending
/// by date, tie-broken via [`super::tag_tiebreak_cmp`] (semver-aware) for
/// same-date tags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagInfo {
    /// Short tag name — e.g. `"v1.0.0"`, not `"refs/tags/v1.0.0"`.
    pub name: String,
    /// Full 40-character SHA-1 hex of the **commit** this tag ultimately
    /// points at. For annotated tags this is the peeled commit (the tag
    /// object itself is not returned). For lightweight tags this is the
    /// commit the ref directly names.
    pub target_rev: String,
    /// For annotated tags: the tagger's timestamp (when `git tag -a` was
    /// run). For lightweight tags: the target commit's committer timestamp.
    pub date: OffsetDateTime,
}
