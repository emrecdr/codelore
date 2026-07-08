//! Repo-layer value types shared across both `GixRepo` and `GitCliRepo`
//! backends and exposed through the `Repo` trait.

use time::OffsetDateTime;

/// A git tag with its resolved target commit OID and the date used for sorting.
///
/// Date semantics follow the git convention for time-ordered tag listing:
/// - **Annotated tags** use the *tagger* timestamp (the date the tag object
///   was created, not when the commit was made).
/// - **Lightweight tags** use the target commit's *committer* timestamp.
///
/// The `Vec<TagInfo>` returned by [`super::Repo::tags`] is sorted ascending
/// by `(date, name)`.
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
