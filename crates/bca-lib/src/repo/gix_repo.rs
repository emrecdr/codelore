//! gix-backed Repo impl. The production default.

use std::path::Path;

use crate::repo::{CommitMetadata, Repo};
use crate::{BcaError, CommitEvent, FileChange, Hunk, Options, Result};

pub struct GixRepo {
    inner: gix::ThreadSafeRepository,
}

impl GixRepo {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let inner = gix::open(path.as_ref())
            .map_err(|e| BcaError::Repo(format!("open {}: {e}", path.as_ref().display())))?
            .into_sync();
        Ok(Self { inner })
    }
}

impl Repo for GixRepo {
    fn walk_commits<'a>(
        &'a self,
        _opts: &'a Options,
    ) -> Result<Box<dyn Iterator<Item = Result<CommitEvent>> + Send + 'a>> {
        let repo = self.inner.to_thread_local();
        let head = repo
            .head_id()
            .map_err(|e| BcaError::Repo(format!("head_id: {e}")))?;

        // Collect OIDs up-front: gix::Repository is !Sync so the Walk iterator
        // cannot be made Send. Collecting OIDs is cheap (they're 20-byte hashes).
        // NOTE(Plan 11): full traversal happens here before any consumer sees commits.
        // When the channel pipeline lands, consider a lazy walk with OIDs collected
        // into a bounded channel instead. Current design is correct for Plan 1.
        let oids: Vec<gix::ObjectId> = repo
            .rev_walk([head])
            .all()
            .map_err(|e| BcaError::Repo(format!("rev_walk: {e}")))?
            .map(|info| match info {
                Ok(i) => Ok(i.id),
                Err(e) => Err(BcaError::Repo(format!("revwalk: {e}"))),
            })
            .collect::<Result<Vec<_>>>()?;

        // The returned iterator owns the OID list and a clone of the Arc-based
        // ThreadSafeRepository — both are Send.
        let inner_clone = self.inner.clone();
        Ok(Box::new(oids.into_iter().map(move |oid| {
            // to_thread_local() is per-iteration because gix::Repository is !Send.
            // The `+ Send` bound on the trait return type forces all captures to be Send,
            // so the Repository must be reconstructed each step from the (Send-able)
            // ThreadSafeRepository clone. Do not hoist this out.
            let repo = inner_clone.to_thread_local();
            let commit = repo
                .find_commit(oid)
                .map_err(|e| BcaError::Repo(format!("find_commit: {e}")))?;
            commit_event_from_gix(&commit)
        })))
    }

    fn changed_files(&self, _rev: &str) -> Result<Vec<FileChange>> {
        // Stub for Plan 1 walking skeleton — Task 9 fills this in via gix tree-diff.
        // The `revisions` analysis (counting commits per HEAD-tracked path)
        // doesn't depend on this for now.
        Ok(vec![])
    }

    fn diff_hunks(&self, _rev: &str, _path: &str) -> Result<Vec<Hunk>> {
        Ok(vec![]) // Task 9 / Plan 4 lands real hunk extraction
    }

    fn resolve_alias(&self, email: &str) -> String {
        // .mailmap support lands in Plan 4 — identity resolution
        email.to_string()
    }

    fn commit_metadata(&self, rev: &str) -> Result<CommitMetadata> {
        Ok(CommitMetadata {
            rev: rev.to_string(),
            signed: false,
            signed_by: None,
            signoffs: vec![],
        })
    }
}

fn commit_event_from_gix(commit: &gix::Commit<'_>) -> Result<CommitEvent> {
    use time::OffsetDateTime;

    let id = commit.id().to_hex().to_string();
    let parents = commit
        .parent_ids()
        .map(|p| p.to_hex().to_string())
        .collect();

    let author_ref = commit
        .author()
        .map_err(|e| BcaError::Repo(format!("author: {e}")))?;
    // CommitEvent only carries committer_email per spec §3.1; we decode the
    // SignatureRef anyway because it's the only way to access the email field.
    let committer_ref = commit
        .committer()
        .map_err(|e| BcaError::Repo(format!("committer: {e}")))?;

    let ts_seconds = author_ref
        .time()
        .map_err(|e| BcaError::Repo(format!("author time: {e}")))?
        .seconds;
    let date = OffsetDateTime::from_unix_timestamp(ts_seconds)
        .map_err(|e| BcaError::Repo(format!("commit timestamp {ts_seconds}: {e}")))?
        .date();

    let message = commit
        .message_raw()
        .map(ToString::to_string)
        .unwrap_or_default();

    Ok(CommitEvent {
        rev: id,
        author_email: author_ref.email.to_string(),
        author_name: author_ref.name.to_string(),
        committer_email: committer_ref.email.to_string(),
        date,
        message,
        parents,
        changes: vec![], // Filled by Task 9 — tree-diff against parent
        kamei: None,
    })
}
