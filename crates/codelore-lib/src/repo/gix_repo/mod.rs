//! gix-backed Repo impl. The production default.

use std::path::Path;

use rayon::iter::{IntoParallelRefIterator, ParallelIterator};

use crate::repo::{BlobReader, Repo};
use crate::{CodeLoreError, CommitEvent, FileChange, Hunk, Options, Result};

mod blob_reader;
mod history;

use blob_reader::GixBlobReader;
use history::{
    WalkerStream, blob_at_path, compute_changed_files, count_loc_and_hunks, process_commit_oid,
};

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
        opts: &'a Options,
    ) -> Result<Box<dyn Iterator<Item = Result<CommitEvent>> + Send + 'a>> {
        let repo = self.inner.to_thread_local();
        let head = repo
            .head_id()
            .map_err(|e| CodeLoreError::Repo(format!("head_id: {e}")))?;

        // Collect OIDs up-front: gix::Repository is !Sync so the Walk iterator
        // cannot be made Send. Collecting OIDs is cheap (they're 20-byte hashes).
        //
        // Apply `Options.after` / `Options.before` (date-range filter) and
        // `Options.include_merges` (merge filter) at this layer so the
        // GixRepo and GitCliRepo backends produce identical event streams
        // for identical Options. Without this, `--after`/`--before` and the
        // default merge-exclusion would silently no-op on the gix backend.
        //
        // NEW-3 perf optimisation: when `--after` is set, switch the walk's
        // sorting to `ByCommitTimeCutoff` so gix stops traversing the commit
        // graph the moment it crosses below the cutoff. Without this, the
        // walker eagerly loaded every reachable commit (~O(history-size))
        // before the in-memory filter dropped most of them — making
        // `--after 2026-06-01` on a 100k-commit repo do the same work as
        // walking all 100k. The cutoff is on committer time (gix's primitive),
        // not author time. The author-time filter inside the closure is
        // kept exact, so commits whose committer time is below the cutoff
        // but author time is above are NOT included (the cutoff is a perf
        // upper bound, not a semantic change — same in-memory predicate
        // wins). Worst-case for unusual rebases that move commit time far
        // earlier than author time: we drop those — but git's own
        // `--after` flag has the same behaviour (it's commit-time based),
        // so GitCliRepo would too. Net: GixRepo and GitCliRepo CONVERGE.
        let mut walk = repo.rev_walk([head]);
        if let Some(after) = opts.after
            && let Ok(start_of_day) = after.with_hms(0, 0, 0)
        {
            // `time::Date::with_hms` only fails on invalid HMS components, so
            // 0,0,0 always succeeds; the `if let Ok` is defensive belt-and-
            // braces. `unix_timestamp()` returns `i64` which is the exact
            // type `gix::revision::walk::Sorting::ByCommitTimeCutoff::seconds`
            // accepts (via the `gix_date::SecondsSinceUnixEpoch = i64` alias),
            // so no conversion is needed.
            let seconds = start_of_day.assume_utc().unix_timestamp();
            walk = walk.sorting(gix::revision::walk::Sorting::ByCommitTimeCutoff {
                seconds,
                order: gix::traverse::commit::simple::CommitTimeOrder::NewestFirst,
            });
        }
        // Collect OIDs WITHOUT parsing commit objects on the main
        // thread. The previous filter pass called `repo.find_commit(oid)`
        // for every reachable commit on the hot path, then `process_commit_oid`
        // called `find_commit` AGAIN on the worker — two object-store lookups
        // per surviving commit, with the first one serialised on a single
        // thread. Filtering is now folded into `process_commit_oid`
        // (returning `Result<Option<CommitEvent>>`), so the OID gather is
        // pure index iteration and filtering parallelises across workers.
        //
        // The rowid-ASC invariant (commits.rowid ASC = gix walk order) is preserved:
        // the OID vec retains walk order, par_iter().collect() preserves
        // per-chunk order, and the driver thread drains Nones without
        // inserting them — so rowid still tracks walk order on the
        // surviving subset.
        let oids: Vec<gix::ObjectId> = walk
            .all()
            .map_err(|e| CodeLoreError::Repo(format!("rev_walk: {e}")))?
            .map(|info| {
                info.map(|i| i.id)
                    .map_err(|e| CodeLoreError::Repo(format!("revwalk: {e}")))
            })
            .collect::<Result<Vec<_>>>()?;
        let filter_include_merges = opts.include_merges;
        let filter_after = opts.after;
        let filter_before = opts.before;

        // Stream events through a bounded crossbeam channel
        // rather than eagerly collecting the full event list into memory.
        // The previous implementation called `par_iter().collect()`
        // which materialised gigabytes on large repos (100k+ commits with
        // rich changes/hunks per commit), bypassed the producer-consumer
        // channel architecture, and could OOM CI runners.
        //
        // The architectural challenge: the `commits.rowid ASC` tiebreak
        // REQUIRES insertion order to match commit-walk order. Pure
        // streaming (`par_iter().for_each(send)`) scrambles order across
        // worker threads and silently breaks that ordering.
        //
        // Resolution: chunked rayon. Process oids in batches of
        // `WALKER_CHUNK_SIZE`, each batch parallelised with
        // order-preserving `collect::<Vec<_>>`, then drained serially
        // through the channel. Order is preserved both within and across
        // chunks (chunks processed sequentially in the driver thread).
        // Peak memory: one chunk's events (~1 MB at 1000 × typical event
        // size) + channel buffer. Bounded regardless of repo size.
        #[allow(clippy::items_after_statements)] // scoped to this function (the only call site); inline placement keeps the const adjacent to the comment that explains its value
        const WALKER_CHUNK_SIZE: usize = 1000;
        #[allow(clippy::items_after_statements)] // same rationale as WALKER_CHUNK_SIZE — single call site, inline scoping keeps both tuning knobs together
        const WALKER_CHANNEL_CAPACITY: usize = 256;

        let inner_clone = self.inner.clone();
        // Parse `.mailmap` ONCE up front; the snapshot is owned bytes
        // (Send + Sync) and is shared across all workers.
        let mailmap = inner_clone.to_thread_local().open_mailmap();
        let (tx, rx) = crossbeam_channel::bounded::<Result<CommitEvent>>(WALKER_CHANNEL_CAPACITY);

        let handle = std::thread::Builder::new()
            .name("codelore-gix-walker".into())
            .spawn(move || {
                for chunk in oids.chunks(WALKER_CHUNK_SIZE) {
                    // Order-preserving parallel map over this chunk. Filter
                    // logic moved INSIDE the worker: each worker opens
                    // the commit object ONCE for both filtering and event
                    // construction; filtered-out commits return Ok(None).
                    let events: Result<Vec<Option<CommitEvent>>> = chunk
                        .par_iter()
                        .map(|oid| {
                            process_commit_oid(
                                *oid,
                                &inner_clone,
                                &mailmap,
                                filter_include_merges,
                                filter_after,
                                filter_before,
                            )
                        })
                        .collect();
                    match events {
                        Ok(events) => {
                            for event in events.into_iter().flatten() {
                                // tx.send returns Err if the receiver was
                                // dropped (consumer abandoned the
                                // iterator). Stop processing and let the
                                // driver thread exit gracefully.
                                if tx.send(Ok(event)).is_err() {
                                    return;
                                }
                            }
                        }
                        Err(e) => {
                            // Send the error and stop. Subsequent rx.next()
                            // will yield Err; downstream caller decides.
                            let _ = tx.send(Err(e));
                            return;
                        }
                    }
                }
                // tx drops when the closure exits — signals end-of-stream
                // to the receiver iterator.
            })
            .map_err(|e| CodeLoreError::Repo(format!("spawn walker thread: {e}")))?;

        // Wrap the receiver in a stream that owns the JoinHandle so a
        // walker-thread panic surfaces as a final `Err` instead of being
        // silently swallowed as clean end-of-stream. Without this, the
        // closure unwinds, `tx` drops, the rx iterator ends, and the
        // caller sees a successful (but truncated) walk. The CLI's
        // typed-error → exit-code chain stays intact via
        // `CodeLoreError::Repo` mapped to exit 3.
        Ok(Box::new(WalkerStream {
            inner: rx.into_iter(),
            handle: Some(handle),
            surfaced_panic: false,
        }))
    }

    fn changed_files(&self, rev: &str) -> Result<Vec<FileChange>> {
        compute_changed_files(&self.inner, rev)
    }

    fn diff_hunks(&self, rev: &str, path: &str) -> Result<Vec<Hunk>> {
        // Resolve the commit + its first parent and look up the
        // before/after blob OIDs for `path`. Root commits (no parent)
        // diff against the empty tree — `count_loc_and_hunks` takes
        // `Option<ObjectId>` precisely so a `None` on either side is
        // interpreted as "empty" without needing a synthetic empty blob.
        let repo = self.inner.to_thread_local();
        let commit_id = repo
            .rev_parse_single(rev)
            .map_err(|e| CodeLoreError::Repo(format!("rev-parse {rev}: {e}")))?;
        let commit = repo
            .find_object(commit_id)
            .map_err(|e| CodeLoreError::Repo(format!("find commit {rev}: {e}")))?
            .into_commit();
        let new_oid = blob_at_path(&repo, &commit, path)?;
        let parent_id_opt = commit.parent_ids().next().map(gix::Id::detach);
        let old_oid = if let Some(parent_id) = parent_id_opt {
            let parent = repo
                .find_object(parent_id)
                .map_err(|e| CodeLoreError::Repo(format!("find parent {parent_id}: {e}")))?
                .into_commit();
            blob_at_path(&repo, &parent, path)?
        } else {
            None
        };
        let (_, _, hunks) = count_loc_and_hunks(&repo, old_oid, new_oid)?;
        Ok(hunks)
    }

    fn resolve_alias(&self, name: &str, email: &str) -> String {
        use gix::bstr::ByteSlice as _;

        let repo = self.inner.to_thread_local();
        let mailmap = repo.open_mailmap();

        // Pass the actual author name (not `b""`) so name+email mailmap
        // entries match. Earlier versions of this method built the
        // SignatureRef with an empty name, which only matched email-only
        // rules — the same blind spot that bit `GitCliRepo::walk_commits`
        // and motivated the trait signature change.
        let sig_ref = gix::actor::SignatureRef {
            name: name.as_bytes().as_bstr(),
            email: email.as_bytes().as_bstr(),
            time: "0 +0000",
        };

        match mailmap.try_resolve(sig_ref) {
            Some(resolved) => resolved.email.to_str().unwrap_or(email).to_string(),
            None => email.to_string(),
        }
    }

    fn head_sha(&self) -> Result<String> {
        let repo = self.inner.to_thread_local();
        let oid = repo
            .head_id()
            .map_err(|e| CodeLoreError::Repo(format!("head_id: {e}")))?;
        Ok(oid.to_hex().to_string())
    }

    fn is_worktree_dirty(&self) -> bool {
        // Tracked-only: `Repository::is_dirty()` compares HEAD-tree-vs-index
        // (staged) and index-vs-worktree (unstaged) while skipping the
        // untracked-file dirwalk entirely — matching `GitCliRepo`'s
        // `--untracked-files=no` porcelain output for ordinary worktrees.
        // Exception: a submodule whose only change is untracked content in its
        // own worktree reports dirty via `is_dirty()` (which does not expose
        // submodule-dirwalk alignment) but clean via `GitCliRepo`. Every caller
        // (the `calibrate-defects` mining guard, the cache-hit staleness
        // warning, the dirty cache-write skip) protects HEAD-time metrics
        // computed over `tracked_paths_at_head()` only, so untracked files
        // must not count.
        //
        // Errors are deliberately swallowed (`unwrap_or(false)`): detection
        // is a hint that triggers a tracing::warn!, not a contract. A
        // missed warning is strictly preferable to a hard analyze failure
        // on a status-API edge case.
        let repo = self.inner.to_thread_local();
        repo.is_dirty().unwrap_or(false)
    }

    fn is_shallow(&self) -> bool {
        // `Repository::is_shallow()` reports a non-empty `shallow` grafts file in
        // the repository's common git dir (worktree-correct via `common_dir()`),
        // exactly the truncated-history signal a `fetch-depth` clone leaves.
        self.inner.to_thread_local().is_shallow()
    }

    fn merge_or_rebase_in_progress(&self) -> bool {
        // `Repository::state()` reproduces git's own `wt-status` probe: it
        // inspects `rebase-apply/`, `rebase-merge/`, `CHERRY_PICK_HEAD`,
        // `MERGE_HEAD`, `BISECT_LOG`, and `REVERT_HEAD` under the (worktree-
        // correct) git dir and reports the in-progress operation. That covers
        // all five markers this method contracts on, so we defer to the
        // library rather than hand-rolling the file checks. The one state it
        // reports that is NOT one of the five is `Bisect` — a bisect is
        // neither a merge nor a rebase, and `GitCliRepo` doesn't probe
        // `BISECT_LOG` — so we exclude it to keep both backends in exact
        // agreement.
        use gix::state::InProgress;
        let repo = self.inner.to_thread_local();
        repo.state()
            .is_some_and(|state| state != InProgress::Bisect)
    }

    fn read_blob_at(&self, rev: &str, path: &str) -> Result<Option<Vec<u8>>> {
        let repo = self.inner.to_thread_local();
        // Resolve `rev` to a single object id. `rev_parse_single` handles
        // "HEAD", full/abbrev SHAs, tags, etc. — the same resolver the
        // commit walker uses, so a SHA from `walk_commits` round-trips.
        let commit_id = repo
            .rev_parse_single(rev)
            .map_err(|e| CodeLoreError::Repo(format!("read_blob_at rev_parse {rev}: {e}")))?;
        let commit = repo
            .find_commit(commit_id)
            .map_err(|e| CodeLoreError::Repo(format!("read_blob_at find_commit {rev}: {e}")))?;
        let tree = commit
            .tree()
            .map_err(|e| CodeLoreError::Repo(format!("read_blob_at tree {rev}: {e}")))?;
        // `lookup_entry_by_path` accepts a POSIX-separated repo-relative
        // string and walks the tree segment by segment. Returns
        // `Ok(None)` if any segment is missing — that's the "not tracked
        // at this rev" case the trait wants surfaced as `Ok(None)`.
        let entry = tree
            .lookup_entry_by_path(path)
            .map_err(|e| CodeLoreError::Repo(format!("read_blob_at lookup {rev}:{path}: {e}")))?;
        let Some(entry) = entry else {
            return Ok(None);
        };
        // Reject non-blob entries (directories, submodule gitlinks, etc.).
        // The scan only cares about file contents.
        if !entry.mode().is_blob() {
            return Ok(None);
        }
        let mut obj = repo.find_object(entry.id()).map_err(|e| {
            CodeLoreError::Repo(format!("read_blob_at find_object {rev}:{path}: {e}"))
        })?;
        // Move the buffer out of the gix::Object via mem::take (gix::Object
        // implements Drop so partial-move isn't permitted). Avoids
        // re-allocating + memcpy'ing up to MAX_DIFF_BLOB_BYTES per file.
        Ok(Some(std::mem::take(&mut obj.data)))
    }

    fn blob_reader_at<'a>(&'a self, rev: &str) -> Box<dyn BlobReader + 'a> {
        Box::new(GixBlobReader::new(&self.inner, rev))
    }

    fn worktree_changes(&self) -> Result<Vec<super::WorktreeChange>> {
        use gix::status::index_worktree::Item as IndexWorktreeItem;
        use gix::status::plumbing::index_as_worktree::{Change as UnstagedChange, EntryStatus};

        let repo = self.inner.to_thread_local();
        let Some(workdir) = repo.workdir().map(std::path::Path::to_path_buf) else {
            return Err(CodeLoreError::Repo(
                "worktree_changes: bare repository has no working tree".into(),
            ));
        };
        // Two streams behind one iterator: `TreeIndex` items are the staged
        // half (HEAD vs index, rename-tracked per `status.renames` /
        // `diff.renames` — the same config `git status` reads), and
        // `IndexWorktree` items are the unstaged half (index vs worktree).
        // `UntrackedFiles::None` disables the directory walk entirely, so
        // untracked files never surface and the only reachable
        // `IndexWorktree` variant is `Modification`. Item ordering is
        // undefined and the same path can appear in both streams, so
        // candidates are merged by path before net classification.
        let items = repo
            .status(gix::progress::Discard)
            .map_err(|e| CodeLoreError::Repo(format!("worktree_changes: status: {e}")))?
            .untracked_files(gix::status::UntrackedFiles::None)
            .index_worktree_submodules(None)
            .into_iter(Vec::new())
            .map_err(|e| CodeLoreError::Repo(format!("worktree_changes: status iter: {e}")))?;

        let mut candidates = std::collections::BTreeMap::new();
        for item in items {
            let item = item
                .map_err(|e| CodeLoreError::Repo(format!("worktree_changes: status item: {e}")))?;
            match item {
                gix::status::Item::TreeIndex(change) => {
                    if is_symlink_or_gitlink(change.entry_mode()) {
                        continue;
                    }
                    if let gix::diff::index::ChangeRef::Rewrite {
                        source_location,
                        location,
                        copy,
                        ..
                    } = &change
                    {
                        if *copy {
                            // A copy's source still exists unchanged, so only
                            // the destination is a candidate.
                            super::add_worktree_candidate(
                                &mut candidates,
                                location.to_string(),
                                None,
                            );
                        } else {
                            let source = source_location.to_string();
                            super::add_worktree_candidate(
                                &mut candidates,
                                location.to_string(),
                                Some(source.clone()),
                            );
                            super::add_worktree_candidate(&mut candidates, source, None);
                        }
                    } else {
                        super::add_worktree_candidate(
                            &mut candidates,
                            change.location().to_string(),
                            None,
                        );
                    }
                }
                gix::status::Item::IndexWorktree(IndexWorktreeItem::Modification {
                    entry,
                    rela_path,
                    status,
                    ..
                }) => {
                    // Conflicts must error regardless of file mode so both
                    // backends agree — the CLI parser rejects `u ` records
                    // before any mode filtering. The symlink/gitlink filter
                    // therefore applies only to genuine content changes.
                    match status {
                        EntryStatus::Conflict { .. } => {
                            return Err(CodeLoreError::Analysis(
                                super::WORKTREE_CONFLICT_MESSAGE.into(),
                            ));
                        }
                        EntryStatus::Change(
                            UnstagedChange::Removed
                            | UnstagedChange::Type { .. }
                            | UnstagedChange::Modification { .. },
                        )
                        | EntryStatus::IntentToAdd => {
                            if !is_symlink_or_gitlink(entry.mode) {
                                super::add_worktree_candidate(
                                    &mut candidates,
                                    rela_path.to_string(),
                                    None,
                                );
                            }
                        }
                        // Submodule status is disabled via
                        // `index_worktree_submodules(None)`; `NeedsUpdate`
                        // means stat-refresh only, not a content change.
                        EntryStatus::Change(UnstagedChange::SubmoduleModification(_))
                        | EntryStatus::NeedsUpdate(_) => {}
                    }
                }
                // With the directory walk disabled there are no
                // directory-contents or worktree-rename items; skip rather
                // than assert unreachable.
                gix::status::Item::IndexWorktree(_) => {}
            }
        }
        super::net_classify_candidates(self, &workdir, candidates)
    }

    fn tracked_paths_at_head(&self) -> Result<Vec<String>> {
        let repo = self.inner.to_thread_local();
        let head_id = repo
            .head_id()
            .map_err(|e| CodeLoreError::Repo(format!("tracked_paths_at_head head_id: {e}")))?;
        let commit = repo
            .find_commit(head_id)
            .map_err(|e| CodeLoreError::Repo(format!("tracked_paths_at_head find_commit: {e}")))?;
        let tree = commit
            .tree()
            .map_err(|e| CodeLoreError::Repo(format!("tracked_paths_at_head tree: {e}")))?;
        // The breadth-first `files()` preset records EVERY reachable entry
        // (tree entries included) with its full repo-relative,
        // `/`-separated path. `is_blob()` keeps regular-file blobs
        // (100644/100755) only — symlinks (120000) and submodule gitlinks
        // (160000) fall outside the blob mode class, matching the trait
        // contract and `read_blob_at`'s `is_blob` guard.
        let entries = tree
            .traverse()
            .breadthfirst
            .files()
            .map_err(|e| CodeLoreError::Repo(format!("tracked_paths_at_head traverse: {e}")))?;
        let mut paths: Vec<String> = entries
            .into_iter()
            .filter(|entry| entry.mode.is_blob())
            .map(|entry| entry.filepath.to_string())
            .collect();
        // Tree traversal yields git tree order (directories sort with a
        // virtual trailing `/`), which differs from plain byte order of
        // full paths. Sort explicitly so both backends return the same
        // deterministic ascending order.
        paths.sort_unstable();
        Ok(paths)
    }

    fn tags(&self) -> Result<Vec<super::TagInfo>> {
        use super::TagInfo;
        use time::OffsetDateTime;

        let repo = self.inner.to_thread_local();
        let mut tags = Vec::new();

        for r in repo
            .references()
            .map_err(|e| CodeLoreError::Repo(format!("references: {e}")))?
            .tags()
            .map_err(|e| CodeLoreError::Repo(format!("tags refs: {e}")))?
        {
            let mut r = r.map_err(|e| CodeLoreError::Repo(format!("ref iter: {e}")))?;

            let name = r.name().shorten().to_string();

            let direct_oid = r
                .try_id()
                .ok_or_else(|| CodeLoreError::Repo(format!("tag {name} is a symbolic ref")))?
                .detach();

            let obj = repo
                .find_object(direct_oid)
                .map_err(|e| CodeLoreError::Repo(format!("find object for tag {name}: {e}")))?;

            let (target_rev, date) = if obj.kind == gix::objs::Kind::Tag {
                // Annotated tag: use the tagger date, peel through to the commit.
                let tag = obj
                    .try_into_tag()
                    .map_err(|e| CodeLoreError::Repo(format!("tag object {name}: {e}")))?;
                let seconds = tag
                    .tagger()
                    .map_err(|e| CodeLoreError::Repo(format!("decode tagger {name}: {e}")))?
                    .ok_or_else(|| {
                        CodeLoreError::Repo(format!("annotated tag {name} has no tagger"))
                    })?
                    .time()
                    .map_err(|e| CodeLoreError::Repo(format!("tagger time {name}: {e}")))?
                    .seconds;
                let date = OffsetDateTime::from_unix_timestamp(seconds)
                    .map_err(|e| CodeLoreError::Repo(format!("tagger timestamp {name}: {e}")))?;
                // peel_to_commit follows any number of nested tag objects.
                let commit_rev = r
                    .peel_to_commit()
                    .map_err(|e| CodeLoreError::Repo(format!("peel to commit {name}: {e}")))?
                    .id()
                    .to_hex()
                    .to_string();
                (commit_rev, date)
            } else {
                // Lightweight tag: ref points directly to a commit object.
                let commit = obj.into_commit();
                let ts_seconds = commit
                    .committer()
                    .map_err(|e| CodeLoreError::Repo(format!("committer for tag {name}: {e}")))?
                    .time()
                    .map_err(|e| {
                        CodeLoreError::Repo(format!("committer time for tag {name}: {e}"))
                    })?
                    .seconds;
                let date = OffsetDateTime::from_unix_timestamp(ts_seconds).map_err(|e| {
                    CodeLoreError::Repo(format!("commit timestamp for tag {name}: {e}"))
                })?;
                (direct_oid.to_hex().to_string(), date)
            };

            tags.push(TagInfo {
                name,
                target_rev,
                date,
            });
        }

        tags.sort_by(|a, b| {
            a.date
                .cmp(&b.date)
                .then_with(|| super::tag_tiebreak_cmp(&a.name, &b.name))
        });
        Ok(tags)
    }
}

impl GixRepo {
    /// Returns the short branch name HEAD currently points at, or `None` when
    /// HEAD is detached (no branch ref). Used by the pre-flight banner.
    ///
    /// Lives as an inherent method (not on the `Repository` trait) because
    /// `GitCliRepo` would have to shell out to `git symbolic-ref` for parity
    /// and we don't need branch-aware behavior in any analysis — this is a
    /// presentation-layer accessor only.
    #[must_use]
    pub fn head_branch_name(&self) -> Option<String> {
        let repo = self.inner.to_thread_local();
        let head = repo.head().ok()?;
        let referent = head.referent_name()?;
        Some(referent.shorten().to_string())
    }
}

/// True for index-entry modes that carry no source bytes the scans can
/// parse: symlinks (`120000`) and submodule gitlinks (`160000`), matched
/// via the file-type class bits so the check mirrors the blob-class
/// (`100xxx`) convention used by `tracked_paths_at_head`. `GitCliRepo`
/// applies the same rule to the porcelain-v2 octal mode fields.
fn is_symlink_or_gitlink(mode: gix::index::entry::Mode) -> bool {
    matches!(mode.bits() & 0o170_000, 0o120_000 | 0o160_000)
}
