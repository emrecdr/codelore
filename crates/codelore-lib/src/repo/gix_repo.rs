//! gix-backed Repo impl. The production default.

use std::path::Path;

use gix::diff::tree_with_rewrites::Change as GixChange;

use crate::repo::{CommitMetadata, Repo};
use crate::{ChangeType, CodeLoreError, CommitEvent, FileChange, Hunk, Options, Result};

pub struct GixRepo {
    inner: gix::ThreadSafeRepository,
}

impl GixRepo {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let inner = gix::open(path.as_ref())
            .map_err(|e| CodeLoreError::Repo(format!("open {}: {e}", path.as_ref().display())))?
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
            .map_err(|e| CodeLoreError::Repo(format!("head_id: {e}")))?;

        // Collect OIDs up-front: gix::Repository is !Sync so the Walk iterator
        // cannot be made Send. Collecting OIDs is cheap (they're 20-byte hashes).
        // NOTE(Plan 4): full traversal happens here before any consumer sees commits.
        // When the channel pipeline lands (Plan 4), consider a lazy walk with OIDs
        // collected into a bounded channel instead. Current design is correct for Plan 1.
        let oids: Vec<gix::ObjectId> = repo
            .rev_walk([head])
            .all()
            .map_err(|e| CodeLoreError::Repo(format!("rev_walk: {e}")))?
            .map(|info| match info {
                Ok(i) => Ok(i.id),
                Err(e) => Err(CodeLoreError::Repo(format!("revwalk: {e}"))),
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
                .map_err(|e| CodeLoreError::Repo(format!("find_commit: {e}")))?;
            let id_string = oid.to_hex().to_string();
            let mut event = commit_event_from_gix(&commit)?;
            event.changes = compute_changed_files(&inner_clone, &id_string)?;

            // Resolve mailmap alias and classify AI attribution in the producer thread
            // (where the Repo is in scope), storing results on the event for the consumer.
            let canonical = {
                use gix::bstr::ByteSlice as _;
                let repo_local = inner_clone.to_thread_local();
                let mailmap = repo_local.open_mailmap();
                let sig_ref = gix::actor::SignatureRef {
                    name: b"".as_bstr(),
                    email: event.author_email.as_bytes().as_bstr(),
                    time: "0 +0000",
                };
                match mailmap.try_resolve(sig_ref) {
                    Some(resolved) => resolved
                        .email
                        .to_str()
                        .unwrap_or(&event.author_email)
                        .to_string(),
                    None => event.author_email.clone(),
                }
            };
            let ai_attr = crate::identity::ai_attribution(
                &event.author_email,
                &event.author_name,
                &event.message,
            );
            event.canonical_author = Some(canonical);
            event.ai_attribution = Some(ai_attr.to_string());

            Ok(event)
        })))
    }

    fn changed_files(&self, rev: &str) -> Result<Vec<FileChange>> {
        compute_changed_files(&self.inner, rev)
    }

    fn diff_hunks(&self, _rev: &str, _path: &str) -> Result<Vec<Hunk>> {
        Ok(vec![]) // Plan 4 lands real hunk extraction
    }

    fn resolve_alias(&self, email: &str) -> String {
        use gix::bstr::ByteSlice as _;

        let repo = self.inner.to_thread_local();
        let mailmap = repo.open_mailmap();

        // Build a minimal SignatureRef: empty name, the email under test, and a
        // dummy time string (gix_actor::SignatureRef::time is &str, not parsed).
        let sig_ref = gix::actor::SignatureRef {
            name: b"".as_bstr(),
            email: email.as_bytes().as_bstr(),
            time: "0 +0000",
        };

        match mailmap.try_resolve(sig_ref) {
            Some(resolved) => resolved.email.to_str().unwrap_or(email).to_string(),
            None => email.to_string(),
        }
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

/// Compute the per-file changes for a single commit identified by `rev`.
///
/// Strategy: parse `rev` → look up commit → get commit tree and parent tree
/// → call `repo.diff_tree_to_tree(parent_tree, commit_tree, options)` which
/// returns `Vec<ChangeDetached>` (= `gix_diff::tree_with_rewrites::Change`).
///
/// For Plan 1 we set `loc_added`/`loc_deleted` to 0 and `hunks` to empty;
/// Plan 4 will add real blob-diff line counting.
fn compute_changed_files(inner: &gix::ThreadSafeRepository, rev: &str) -> Result<Vec<FileChange>> {
    let repo = inner.to_thread_local();

    // Parse the rev string to an ObjectId.
    let oid = rev
        .parse::<gix::ObjectId>()
        .map_err(|e| CodeLoreError::Repo(format!("parse rev {rev:?}: {e}")))?;

    let commit = repo
        .find_commit(oid)
        .map_err(|e| CodeLoreError::Repo(format!("find_commit {rev}: {e}")))?;

    let tree = commit
        .tree()
        .map_err(|e| CodeLoreError::Repo(format!("commit tree {rev}: {e}")))?;

    // Get the first parent's tree, or use the empty tree for root commits.
    let parent_tree = commit
        .parent_ids()
        .next()
        .map(|pid| {
            let parent_commit = repo
                .find_commit(pid)
                .map_err(|e| CodeLoreError::Repo(format!("find_parent_commit {rev}: {e}")))?;
            parent_commit
                .tree()
                .map_err(|e| CodeLoreError::Repo(format!("parent tree {rev}: {e}")))
        })
        .transpose()?;

    // Configure diff options: track full paths, no rewrite detection for Plan 1.
    let mut diff_opts = gix::diff::Options::default();
    diff_opts.track_rewrites(None);

    let changes: Vec<_> = repo
        .diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), diff_opts)
        .map_err(|e| CodeLoreError::Repo(format!("diff_tree_to_tree {rev}: {e}")))?;

    let file_changes = changes
        .into_iter()
        .filter_map(|change| gix_change_to_file_change(change, rev))
        .collect();

    Ok(file_changes)
}

/// Convert a `gix_diff::tree_with_rewrites::Change` to our `FileChange`.
/// Returns `None` for tree entries (directories) — we only care about blobs.
fn gix_change_to_file_change(change: GixChange, _rev: &str) -> Option<FileChange> {
    use gix::objs::tree::EntryKind;

    let is_blob = |mode: gix::objs::tree::EntryMode| {
        matches!(mode.kind(), EntryKind::Blob | EntryKind::BlobExecutable)
    };

    match change {
        GixChange::Addition {
            location,
            entry_mode,
            ..
        } if is_blob(entry_mode) => Some(FileChange {
            path: location.to_string(),
            change_type: ChangeType::Added,
            loc_added: 0,
            loc_deleted: 0,
            hunks: vec![],
        }),
        GixChange::Deletion {
            location,
            entry_mode,
            ..
        } if is_blob(entry_mode) => Some(FileChange {
            path: location.to_string(),
            change_type: ChangeType::Deleted,
            loc_added: 0,
            loc_deleted: 0,
            hunks: vec![],
        }),
        GixChange::Modification {
            location,
            entry_mode,
            ..
        } if is_blob(entry_mode) => Some(FileChange {
            path: location.to_string(),
            change_type: ChangeType::Modified,
            loc_added: 0,
            loc_deleted: 0,
            hunks: vec![],
        }),
        GixChange::Rewrite {
            location,
            source_location,
            entry_mode,
            copy,
            ..
        } if is_blob(entry_mode) => {
            let path = location.to_string();
            let from = source_location.to_string();
            // Sentinel similarity for Plan 1; Plan 4 wires real similarity from `diff` field.
            let change_type = if copy {
                ChangeType::Copied {
                    from,
                    similarity: 100,
                }
            } else {
                ChangeType::Renamed {
                    from,
                    similarity: 100,
                }
            };
            Some(FileChange {
                path,
                change_type,
                loc_added: 0,
                loc_deleted: 0,
                hunks: vec![],
            })
        }
        // Skip non-blob entries (trees / submodules / symlinks treated as non-blob).
        _ => None,
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
        .map_err(|e| CodeLoreError::Repo(format!("author: {e}")))?;
    // CommitEvent only carries committer_email per spec §3.1; we decode the
    // SignatureRef anyway because it's the only way to access the email field.
    let committer_ref = commit
        .committer()
        .map_err(|e| CodeLoreError::Repo(format!("committer: {e}")))?;

    let ts_seconds = author_ref
        .time()
        .map_err(|e| CodeLoreError::Repo(format!("author time: {e}")))?
        .seconds;
    let date = OffsetDateTime::from_unix_timestamp(ts_seconds)
        .map_err(|e| CodeLoreError::Repo(format!("commit timestamp {ts_seconds}: {e}")))?
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
        changes: vec![],        // Populated by the caller; see walk_commits.
        canonical_author: None, // Populated by walk_commits after mailmap resolution.
        ai_attribution: None,   // Populated by walk_commits after identity classification.
        kamei: None,
    })
}
