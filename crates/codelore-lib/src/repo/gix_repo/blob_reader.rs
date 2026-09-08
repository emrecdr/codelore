//! `GixRepo`'s warm-cache override of [`crate::repo::BlobReader`]. Builds one
//! `gix::Repository` (with its own warm object-decode cache) per reader,
//! reused across every `read` call — unlike `GixRepo::read_blob_at`, which
//! calls `to_thread_local()` fresh (a cold decode cache) on every single
//! call.
//!
//! `gix::Tree<'repo>` borrows the `Repository` it came from, so it can't be
//! stored alongside an owned `Repository` in the same struct (that would be
//! self-referential). Instead this caches the resolved root tree's
//! `ObjectId` (an owned, `Copy` value) and re-derives a `Tree` handle from it
//! via `Repository::find_tree` on every `read` — that lookup is a cache HIT
//! against the reader's warm `Repository` after the first call (`find_tree`
//! → `find_object`, which consults the same decode cache
//! `lookup_entry_by_path`'s per-segment walk also uses), so the expensive
//! part — resolving `rev` → commit → root tree via `rev_parse_single` +
//! `find_commit` + `commit.tree()` — still happens exactly once per reader.

use crate::repo::BlobReader;
use crate::{CodeLoreError, Result};

pub(super) struct GixBlobReader {
    /// Owned, warm-cache repository handle — built once per reader (i.e.
    /// once per rayon worker via `map_init`), reused across every `read`.
    repo: gix::Repository,
    rev: String,
    /// The resolved root tree's id, cached after the first successful
    /// resolution. `None` until then; resolution is retried (not cached as
    /// an error) on a rev that fails to resolve, matching `read_blob_at`'s
    /// per-call behavior for that rare path.
    root_tree_id: Option<gix::ObjectId>,
}

impl GixBlobReader {
    pub(super) fn new(inner: &gix::ThreadSafeRepository, rev: &str) -> Self {
        Self {
            repo: inner.to_thread_local(),
            rev: rev.to_string(),
            root_tree_id: None,
        }
    }

    /// Resolve `self.rev` → commit → root tree id, exactly the same
    /// three-step chain `GixRepo::read_blob_at` runs per call, plus the same
    /// error text — so a caller matching on the error message sees identical
    /// output whether it went through this reader or the direct method.
    fn resolve_root_tree_id(&self) -> Result<gix::ObjectId> {
        let commit_id = self.repo.rev_parse_single(self.rev.as_str()).map_err(|e| {
            CodeLoreError::Repo(format!("read_blob_at rev_parse {}: {e}", self.rev))
        })?;
        let commit = self.repo.find_commit(commit_id).map_err(|e| {
            CodeLoreError::Repo(format!("read_blob_at find_commit {}: {e}", self.rev))
        })?;
        let tree = commit
            .tree()
            .map_err(|e| CodeLoreError::Repo(format!("read_blob_at tree {}: {e}", self.rev)))?;
        Ok(tree.id)
    }

    /// Resolve `path` to the id of the blob it names at this reader's rev,
    /// or `None` when the path is not a tracked blob there.
    ///
    /// Shared by [`BlobReader::read`] and [`BlobReader::read_capped`] so the
    /// two can never disagree about which paths exist: everything up to the
    /// point of touching the object's bytes is this one walk, and the two
    /// callers differ only in what they ask for once they hold the id.
    fn blob_id(&mut self, path: &str) -> Result<Option<gix::ObjectId>> {
        let tree_id = if let Some(id) = self.root_tree_id {
            id
        } else {
            let id = self.resolve_root_tree_id()?;
            self.root_tree_id = Some(id);
            id
        };
        // `find_tree` decodes the root tree object — a warm-cache hit on
        // every call after the first, since `self.repo`'s decode cache
        // persists for the reader's whole life (unlike `read_blob_at`,
        // which starts from a brand-new cold cache every call).
        let tree = self
            .repo
            .find_tree(tree_id)
            .map_err(|e| CodeLoreError::Repo(format!("read_blob_at tree {}: {e}", self.rev)))?;
        // Same segment-by-segment walk as `read_blob_at` — `None` on any
        // missing segment is the "not tracked at this rev" case.
        let entry = tree.lookup_entry_by_path(path).map_err(|e| {
            CodeLoreError::Repo(format!("read_blob_at lookup {}:{path}: {e}", self.rev))
        })?;
        let Some(entry) = entry else {
            return Ok(None);
        };
        // Reject non-blob entries (directories, submodule gitlinks, etc.) —
        // same guard as `read_blob_at`.
        if !entry.mode().is_blob() {
            return Ok(None);
        }
        Ok(Some(entry.id().detach()))
    }
}

impl BlobReader for GixBlobReader {
    fn read(&mut self, path: &str) -> Result<Option<Vec<u8>>> {
        let Some(id) = self.blob_id(path)? else {
            return Ok(None);
        };
        let mut obj = self.repo.find_object(id).map_err(|e| {
            CodeLoreError::Repo(format!("read_blob_at find_object {}:{path}: {e}", self.rev))
        })?;
        Ok(Some(std::mem::take(&mut obj.data)))
    }

    /// Consults the object header before deciding, so an oversize blob is
    /// rejected without its bytes ever being allocated — the same
    /// `find_header` probe the history walker uses to keep a large diff blob
    /// out of memory.
    ///
    /// A header lookup that fails falls through to the read path rather than
    /// erroring: the size check there still holds the line, so a backend
    /// quirk costs the allocation this was avoiding rather than the file.
    fn read_capped(&mut self, path: &str, cap: u64) -> Result<Option<crate::repo::BlobRead>> {
        let Some(id) = self.blob_id(path)? else {
            return Ok(None);
        };
        if let Ok(header) = self.repo.find_header(id)
            && header.size() > cap
        {
            return Ok(Some(crate::repo::BlobRead::Oversize(header.size())));
        }
        let mut obj = self.repo.find_object(id).map_err(|e| {
            CodeLoreError::Repo(format!("read_blob_at find_object {}:{path}: {e}", self.rev))
        })?;
        let bytes = std::mem::take(&mut obj.data);
        let len = bytes.len() as u64;
        Ok(Some(if len > cap {
            crate::repo::BlobRead::Oversize(len)
        } else {
            crate::repo::BlobRead::Data(bytes)
        }))
    }
}

#[cfg(all(test, feature = "test-support"))]
mod tests {
    use crate::repo::{GixRepo, Repo};

    /// The header-probe `read_capped` override must agree with `read` about
    /// every path: which paths exist, how large each blob is, and what its
    /// bytes are. The probe skips materialising oversize blobs, so a probe
    /// that reported the wrong size — or disagreed about existence — would
    /// silently drop real files from the HEAD scan.
    #[test]
    fn capped_read_agrees_with_read_on_size_bytes_and_existence() {
        use crate::repo::BlobRead;

        let fixture = crate::test_support::differential_repo::build();
        let repo = GixRepo::open(fixture.dir.path()).expect("open repo");
        let head_rev = repo.head_sha().expect("head_sha");
        let tracked = repo.tracked_paths_at_head().expect("tracked paths");
        assert!(!tracked.is_empty(), "fixture must have tracked files");

        let nested = tracked
            .iter()
            .find(|p| p.contains('/'))
            .expect("fixture should have a file nested in a directory");
        let dir = &nested[..nested.rfind('/').unwrap()];

        let mut reader = repo.blob_reader_at(&head_rev);
        let mut saw_nonempty = false;
        for path in &tracked {
            let truth = reader.read(path).expect("read").expect("tracked blob");

            // Generous cap: must hand back exactly the same bytes.
            match reader.read_capped(path, u64::MAX).expect("capped read") {
                Some(BlobRead::Data(b)) => assert_eq!(b, truth, "byte mismatch for {path}"),
                other => panic!("{path} must read as Data under an unlimited cap, got {other:?}"),
            }

            // Cap below the real size: must report oversize, and report the
            // true length — that number is what the scan logs and what proves
            // the header probe is not guessing.
            if truth.is_empty() {
                continue;
            }
            saw_nonempty = true;
            let cap = (truth.len() - 1) as u64;
            match reader.read_capped(path, cap).expect("capped read") {
                Some(BlobRead::Oversize(size)) => {
                    assert_eq!(size, truth.len() as u64, "wrong reported size for {path}");
                }
                other => panic!("{path} must read as Oversize under a cap below it, got {other:?}"),
            }
        }
        assert!(
            saw_nonempty,
            "fixture must contain at least one non-empty file, or the oversize arm never ran"
        );

        // Non-blob and absent paths resolve the same way as `read`.
        for path in [dir, "this-path-does-not-exist-anywhere.zzz"] {
            assert!(
                reader
                    .read_capped(path, u64::MAX)
                    .expect("capped")
                    .is_none(),
                "{path} must be None, matching read()"
            );
            assert!(reader.read(path).expect("read").is_none(), "{path} sanity");
        }
    }

    /// `blob_reader_at(rev).read(path)` must return byte-identical
    /// `Ok(Some)`/`Ok(None)` to `read_blob_at(rev, path)` for every tracked
    /// path at HEAD, a directory path (non-blob tree entry), a nonexistent
    /// path, and an explicit non-HEAD rev — the warm reader is a caching
    /// wrapper, never a behavior change.
    #[test]
    fn warm_reader_matches_read_blob_at_for_every_path_kind() {
        let fixture = crate::test_support::differential_repo::build();
        let repo = GixRepo::open(fixture.dir.path()).expect("open repo");
        let head_rev = repo.head_sha().expect("head_sha");

        let tracked = repo.tracked_paths_at_head().expect("tracked paths");
        assert!(
            !tracked.is_empty(),
            "fixture must have tracked files at HEAD"
        );

        // A directory path, derived from a nested tracked file so the test
        // adapts to the fixture layout (mirrors the differential-test
        // idiom in `tests/differential_repo_test.rs`).
        let nested = tracked
            .iter()
            .find(|p| p.contains('/'))
            .expect("fixture should have a file nested in a directory");
        let dir = &nested[..nested.rfind('/').unwrap()];

        let mut reader = repo.blob_reader_at(&head_rev);
        for path in tracked
            .iter()
            .map(String::as_str)
            .chain([dir, "this-path-does-not-exist-anywhere.zzz"])
        {
            let direct = repo.read_blob_at(&head_rev, path);
            let warm = reader.read(path);
            match (direct, warm) {
                (Ok(d), Ok(w)) => assert_eq!(d, w, "byte mismatch for {path}"),
                (Err(d), Err(w)) => assert_eq!(
                    d.to_string(),
                    w.to_string(),
                    "error text mismatch for {path}"
                ),
                (d, w) => panic!("Ok/Err variant mismatch for {path}: direct={d:?} warm={w:?}"),
            }
        }

        // An explicit non-HEAD rev too, not just the "HEAD" literal — the
        // reader resolves whatever `rev` string it's given.
        let db = crate::facts::FactsDb::new_in_memory().expect("db");
        let opts = crate::test_support::permissive_coupling_opts(fixture.dir.path().to_path_buf());
        db.ingest(&repo, &opts).expect("ingest");
        let old_rev: String = db
            .query_row(
                "SELECT rev FROM commits ORDER BY date ASC, rowid ASC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .expect("query oldest rev");
        let mut old_reader = repo.blob_reader_at(&old_rev);
        for path in &tracked {
            let direct = repo.read_blob_at(&old_rev, path);
            let warm = old_reader.read(path);
            match (direct, warm) {
                (Ok(d), Ok(w)) => assert_eq!(d, w, "old-rev byte mismatch for {path}"),
                (Err(_), Err(_)) => {}
                (d, w) => {
                    panic!("old-rev Ok/Err variant mismatch for {path}: direct={d:?} warm={w:?}")
                }
            }
        }
    }
}
