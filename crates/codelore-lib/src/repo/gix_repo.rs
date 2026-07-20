//! gix-backed Repo impl. The production default.

use std::path::Path;

use gix::diff::tree_with_rewrites::Change as GixChange;
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};

use crate::repo::Repo;
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

/// Compute the per-file changes for a single commit identified by `rev`.
///
/// Strategy: parse `rev` → look up commit → get commit tree and parent tree
/// → call `repo.diff_tree_to_tree(parent_tree, commit_tree, options)` which
/// returns `Vec<ChangeDetached>` (= `gix_diff::tree_with_rewrites::Change`).
///
/// Modifications carry real `loc_added`/`loc_deleted` + hunks, computed by
/// `changed_files_for_commit` via `count_loc_and_hunks`; pure
/// adds/deletes/type-changes carry empty hunks.
fn compute_changed_files(inner: &gix::ThreadSafeRepository, rev: &str) -> Result<Vec<FileChange>> {
    let repo = inner.to_thread_local();

    // Parse the rev string to an ObjectId.
    let oid = rev
        .parse::<gix::ObjectId>()
        .map_err(|e| CodeLoreError::Repo(format!("parse rev {rev:?}: {e}")))?;

    let commit = repo
        .find_commit(oid)
        .map_err(|e| CodeLoreError::Repo(format!("find_commit {rev}: {e}")))?;
    changed_files_for_commit(&repo, &commit, rev)
}

/// Iterator-step entry point: takes an already-resolved `gix::Commit` so
/// `walk_commits` doesn't have to call `find_commit` twice per commit
/// (once for `CommitEvent` metadata, once again here). The public
/// `compute_changed_files` (used by the `Repo::changed_files` trait
/// method) is a thin wrapper that resolves the OID first.
fn changed_files_for_commit(
    repo: &gix::Repository,
    commit: &gix::Commit<'_>,
    rev: &str,
) -> Result<Vec<FileChange>> {
    // Merge commits (≥2 parents) emit empty change sets to match
    // `git log --name-status`'s default behaviour, which suppresses
    // merge diffs unless `-m` / `-c` / `--cc` is passed. GitCliRepo
    // inherits that suppression for free; without this guard the gix
    // backend reports a first-parent diff for every merge and the
    // differential test gate diverges.
    if commit.parent_ids().count() > 1 {
        return Ok(Vec::new());
    }

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

    // Enable rename tracking with Git's default thresholds (50% similarity,
    // 1000-file fuzzy-match limit, copies OFF). GitCliRepo gets rename
    // detection for free via `git log --name-status` (Git's default `-M`
    // kicks in); leaving GixRepo without it produced divergent change-type
    // values for the same commit — renames showed up as Delete+Add pairs
    // and split a file's history. Copies are off intentionally to match
    // Git's default (`-C` not passed) and keep the two walker backends
    // bit-equivalent for the differential parity tests.
    let mut diff_opts = gix::diff::Options::default();
    diff_opts.track_rewrites(Some(gix::diff::Rewrites::default()));

    let changes: Vec<_> = repo
        .diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), diff_opts)
        .map_err(|e| CodeLoreError::Repo(format!("diff_tree_to_tree {rev}: {e}")))?;

    let file_changes: Result<Vec<FileChange>> = changes
        .into_iter()
        .filter_map(|change| gix_change_to_file_change(change, repo).transpose())
        .collect();

    file_changes
}

/// Convert a `gix_diff::tree_with_rewrites::Change` to our `FileChange`.
/// Returns `Ok(None)` for tree entries / non-blob modes (we only care about
/// blobs) and `Err` if a blob lookup or line-count diff fails.
fn gix_change_to_file_change(
    change: GixChange,
    repo: &gix::Repository,
) -> Result<Option<FileChange>> {
    use gix::objs::tree::EntryKind;

    let is_blob = |mode: gix::objs::tree::EntryMode| {
        matches!(mode.kind(), EntryKind::Blob | EntryKind::BlobExecutable)
    };

    match change {
        GixChange::Addition {
            location,
            entry_mode,
            id,
            ..
        } if is_blob(entry_mode) => {
            let (loc_added, loc_deleted) = count_loc(repo, None, Some(id))?;
            Ok(Some(FileChange {
                path: location.to_string(),
                change_type: ChangeType::Added,
                loc_added,
                loc_deleted,
                hunks: vec![],
            }))
        }
        GixChange::Deletion {
            location,
            entry_mode,
            id,
            ..
        } if is_blob(entry_mode) => {
            let (loc_added, loc_deleted) = count_loc(repo, Some(id), None)?;
            Ok(Some(FileChange {
                path: location.to_string(),
                change_type: ChangeType::Deleted,
                loc_added,
                loc_deleted,
                hunks: vec![],
            }))
        }
        GixChange::Modification {
            location,
            entry_mode,
            previous_id,
            id,
            ..
        } if is_blob(entry_mode) => {
            // Modifications carry hunks: the gix-diff machinery that
            // computes loc_added/loc_deleted ALSO walks the diff hunks
            // for free. We extract both in a single pass.
            let (loc_added, loc_deleted, hunks) =
                count_loc_and_hunks(repo, Some(previous_id), Some(id))?;
            Ok(Some(FileChange {
                path: location.to_string(),
                change_type: ChangeType::Modified,
                loc_added,
                loc_deleted,
                hunks,
            }))
        }
        GixChange::Rewrite {
            location,
            source_location,
            entry_mode,
            diff,
            copy,
            ..
        } if is_blob(entry_mode) => {
            let path = location.to_string();
            let from = source_location.to_string();
            // gix's rewrite tracker already computed similarity + line counts
            // when it diffed source to destination. Prefer those — they're
            // free. If `diff` is `None` (perfect 100% rename), all counts
            // are zero and similarity is 100.
            let (similarity, loc_added, loc_deleted) = if let Some(stats) = diff {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                // explicit .clamp(0.0, 100.0) guarantees the value is in [0,100] before casting to u8; truncation and sign loss are impossible
                let sim = (stats.similarity * 100.0).round().clamp(0.0, 100.0) as u8;
                (sim, stats.insertions, stats.removals)
            } else {
                // No diff means source and destination blobs were
                // bit-identical (gix-diff documents this exact contract).
                // No need to read the blobs and run a histogram pass —
                // the result is always (100% similarity, 0 added, 0 removed).
                (100u8, 0, 0)
            };
            let change_type = if copy {
                ChangeType::Copied { from, similarity }
            } else {
                ChangeType::Renamed { from, similarity }
            };
            Ok(Some(FileChange {
                path,
                change_type,
                loc_added,
                loc_deleted,
                hunks: vec![],
            }))
        }
        // Skip non-blob entries (trees / submodules / symlinks treated as non-blob).
        _ => Ok(None),
    }
}

/// Count added and deleted lines between two blob OIDs.
///
/// Symmetric in old/new — `None` on either side is treated as an empty
/// blob, so Additions diff `""` against the new blob (all lines added)
/// and Deletions diff the old blob against `""` (all lines deleted).
/// Modifications and content-changing Rewrites diff old against new.
///
/// Uses Git's default histogram algorithm via `gix_diff::blob`, which
/// re-exports `imara-diff`. Slider heuristics are applied (`postprocess_lines`)
/// so hunk boundaries match `git diff` output line-for-line — the values
/// here are bit-equivalent to `git log --numstat`.
/// Maximum blob size that `count_loc` will diff. Blobs larger than this on
/// either side return `(0, 0)` to mirror `git log --numstat`'s `- -`
/// behaviour for binary/oversized files. 1 MiB matches Git's
/// `core.bigFileThreshold` default and keeps the histogram diff bounded
/// on commits that touch `SQLite` databases, vendored bundles, or large
/// snapshot fixtures.
const MAX_DIFF_BLOB_BYTES: usize = 1024 * 1024;

/// Window size for git-style binary detection. Git inspects the first
/// 8 KiB of a blob and treats it as binary if any NUL byte is found.
/// Same heuristic — matching `GitCliRepo`'s implicit behaviour (it
/// receives `- -` from `git diff --numstat` for binary files).
const BINARY_SNIFF_BYTES: usize = 8000;

/// Blobs larger than [`MAX_DIFF_BLOB_BYTES`] on either side, or
/// containing a NUL byte in the first [`BINARY_SNIFF_BYTES`], return
/// `(0, 0)` without ever loading the full bytes into `InternedInput` or
/// running the histogram diff. Previously, `count_loc` blindly read raw
/// bytes for any oid: a single commit touching a 50 MiB `SQLite` database
/// allocated 100 MiB of `Vec<u8>` per worker thread and spent seconds in
/// imara-diff on noise (random newline bytes), polluting hotspots /
/// churn / code-health analyses with garbage `loc_added` /
/// `loc_deleted` numbers. `GitCliRepo` doesn't have this problem
/// because git's own `--numstat` filters binary files with `- -`; this
/// fix brings the gix backend into convergence.
fn count_loc(
    repo: &gix::Repository,
    old_oid: Option<gix::ObjectId>,
    new_oid: Option<gix::ObjectId>,
) -> Result<(u32, u32)> {
    let (added, removed, _) = count_loc_and_hunks(repo, old_oid, new_oid)?;
    Ok((added, removed))
}

/// `count_loc` + the per-hunk `(old_start, old_lines, new_start, new_lines)`
/// rows that drive the `hunks` table. The histogram diff is the
/// expensive step (already paid for `loc_added` / `loc_deleted`); the
/// hunk iterator is essentially free on top — `imara_diff::Diff` keeps
/// the line-range ops in memory and yields them via `.hunks()`. So one
/// blob diff produces both metrics + hunks; no second pass.
///
/// Returns `(loc_added, loc_deleted, hunks)`. For binary / oversized
/// blobs and pure adds/deletes the hunks vector is empty (the simple
/// "every line of one side is added/removed" case doesn't carry
/// useful per-hunk granularity).
fn count_loc_and_hunks(
    repo: &gix::Repository,
    old_oid: Option<gix::ObjectId>,
    new_oid: Option<gix::ObjectId>,
) -> Result<(u32, u32, Vec<Hunk>)> {
    use gix::diff::blob::{Algorithm, InternedInput, diff_with_slider_heuristics};

    let empty: Vec<u8> = Vec::new();
    let read_blob = |oid: gix::ObjectId| -> Result<Vec<u8>> {
        let mut obj = repo
            .find_object(oid)
            .map_err(|_e| CodeLoreError::BlobNotFound {
                oid: oid.to_string(),
            })?;
        Ok(std::mem::take(&mut obj.data))
    };

    let is_binary_or_oversized = |bytes: &[u8]| -> bool {
        if bytes.len() > MAX_DIFF_BLOB_BYTES {
            return true;
        }
        let sniff_end = bytes.len().min(BINARY_SNIFF_BYTES);
        bytes[..sniff_end].contains(&0u8)
    };

    let old_bytes = match old_oid {
        Some(oid) => read_blob(oid)?,
        None => empty.clone(),
    };
    let new_bytes = match new_oid {
        Some(oid) => read_blob(oid)?,
        None => empty.clone(),
    };

    if is_binary_or_oversized(&old_bytes) || is_binary_or_oversized(&new_bytes) {
        return Ok((0, 0, Vec::new()));
    }

    // Pure add / pure delete short-circuits skip the InternedInput
    // tokenization. They don't produce per-hunk rows: with one side
    // empty, the diff is a single trivial "everything was
    // added/removed" range that adds no information beyond the
    // already-stored `change_type` + `loc_added` / `loc_deleted` row
    // in `changes`. Leaving `hunks` empty here matches what
    // `git show -p` emits for additions/deletions (a single `@@ -0,0
    // +1,N @@` or `@@ -1,N +0,0 @@` header) and keeps the table from
    // accumulating one-row-per-file for those cases where the data
    // is redundant.
    if old_oid.is_none() {
        return Ok((count_lines(&new_bytes), 0, Vec::new()));
    }
    if new_oid.is_none() {
        return Ok((0, count_lines(&old_bytes), Vec::new()));
    }

    let input = InternedInput::new(old_bytes.as_slice(), new_bytes.as_slice());
    let diff = diff_with_slider_heuristics(Algorithm::Histogram, &input);
    #[allow(clippy::cast_possible_truncation)]
    // imara-diff addition counts are well below u32::MAX (4 billion lines) for any real source file
    let added = diff.count_additions() as u32;
    #[allow(clippy::cast_possible_truncation)]
    // imara-diff removal counts are well below u32::MAX (4 billion lines) for any real source file
    let removed = diff.count_removals() as u32;
    // `Diff::hunks()` is a free walk over the already-computed change
    // regions in the diff — no second diff pass. Each `imara_diff::Hunk`
    // carries `before: Range<u32>` / `after: Range<u32>` (0-indexed,
    // half-open). Convert to git's hunk-header convention so the
    // gix backend's output stays comparable to `GitCliRepo`'s parsed
    // `@@ -old_start,old_lines +new_start,new_lines @@` lines:
    //
    //   - `lines = range.end - range.start`
    //   - `start = range.start + 1` for non-empty sides (1-indexed)
    //   - `start = range.start    ` for empty   sides (git renders
    //     the line-BEFORE-which the change is inserted / from which
    //     lines were removed)
    //
    // Differential-test invariant: parse `git show -p --unified=0`
    // output via `parse_hunk_headers`, run gix-side extraction over
    // the same commit×file, assert equal. The conversion above is
    // exactly what `git diff --unified=0` emits.
    let hunks = diff
        .hunks()
        .map(|h| {
            let old_lines = h.before.end.saturating_sub(h.before.start);
            let new_lines = h.after.end.saturating_sub(h.after.start);
            let old_start = if old_lines == 0 {
                h.before.start
            } else {
                h.before.start + 1
            };
            let new_start = if new_lines == 0 {
                h.after.start
            } else {
                h.after.start + 1
            };
            Hunk {
                old_start,
                old_lines,
                new_start,
                new_lines,
            }
        })
        .collect();
    Ok((added, removed, hunks))
}

/// Resolve `path` to a blob `ObjectId` in `commit`'s tree, or `None` if
/// the path doesn't exist in that tree (i.e. the file was added in the
/// commit and the lookup is against the parent, or the file was deleted
/// in the commit and the lookup is against the head — both `None`
/// branches feed `count_loc_and_hunks` which interprets them as
/// "empty side"). Used by `GixRepo::diff_hunks` to resolve the
/// before/after blob OIDs for a (rev, path) pair.
fn blob_at_path(
    repo: &gix::Repository,
    commit: &gix::Commit<'_>,
    path: &str,
) -> Result<Option<gix::ObjectId>> {
    let tree = commit
        .tree()
        .map_err(|e| CodeLoreError::Repo(format!("commit tree: {e}")))?;
    match tree.lookup_entry_by_path(path) {
        Ok(Some(entry)) => {
            let oid = entry.id().detach();
            let _ = repo; // tree-lookup doesn't need the repo handle directly
            Ok(Some(oid))
        }
        Ok(None) => Ok(None),
        Err(e) => Err(CodeLoreError::Repo(format!("lookup `{path}` in tree: {e}"))),
    }
}

/// Count line terminators (LF) in a byte slice, capping at `u32::MAX`.
/// Matches imara-diff's tokenization on pure-add / pure-delete sides:
/// each `\n`-terminated segment is one line. A trailing partial line
/// (no final `\n`) is also counted, mirroring how the histogram diff
/// would interpret it.
fn count_lines(bytes: &[u8]) -> u32 {
    use gix::bstr::ByteSlice as _;
    if bytes.is_empty() {
        return 0;
    }
    let nl = bytes.find_iter(b"\n").count();
    let total = if bytes.last() == Some(&b'\n') {
        nl
    } else {
        nl + 1
    };
    u32::try_from(total).unwrap_or(u32::MAX)
}

/// Extract the author-time as a full `OffsetDateTime` from a gix commit.
/// Used by both the `CommitEvent` constructor and the pre-filter in
/// `walk_commits`. Returns UTC-anchored (`from_unix_timestamp` always
/// yields a UTC instant); the original author tz offset from the gix
/// signature is currently discarded — see schema v2 doc for the
/// tz-preservation roadmap.
fn commit_author_date(commit: &gix::Commit<'_>) -> Result<time::OffsetDateTime> {
    use time::OffsetDateTime;
    let author_ref = commit
        .author()
        .map_err(|e| CodeLoreError::Repo(format!("author: {e}")))?;
    let ts_seconds = author_ref
        .time()
        .map_err(|e| CodeLoreError::Repo(format!("author time: {e}")))?
        .seconds;
    OffsetDateTime::from_unix_timestamp(ts_seconds)
        .map_err(|e| CodeLoreError::Repo(format!("commit timestamp {ts_seconds}: {e}")))
}

/// Extract the committer-time as a full `OffsetDateTime`. Mirrors
/// `commit_author_date` but reads the committer signature. The delta
/// `(committer_date - date)` is the in-flight time the `lead-time`
/// and `delivery-friction` analyses surface. Same tz-stripping
/// behaviour as the author variant.
fn commit_committer_date(commit: &gix::Commit<'_>) -> Result<time::OffsetDateTime> {
    use time::OffsetDateTime;
    let committer_ref = commit
        .committer()
        .map_err(|e| CodeLoreError::Repo(format!("committer: {e}")))?;
    let ts_seconds = committer_ref
        .time()
        .map_err(|e| CodeLoreError::Repo(format!("committer time: {e}")))?
        .seconds;
    OffsetDateTime::from_unix_timestamp(ts_seconds)
        .map_err(|e| CodeLoreError::Repo(format!("committer timestamp {ts_seconds}: {e}")))
}

/// Extract a fully-resolved `CommitEvent` from a single oid.
/// Called by every rayon worker in the chunked walker — each worker
/// constructs its own thread-local `gix::Repository` from the shared
/// `ThreadSafeRepository` clone, finds the commit, computes changes,
/// resolves mailmap canonical author, and classifies AI attribution.
///
/// Free function (not a closure) so the chunked rayon driver can call it
/// directly without dragging closure-capture lifetimes through the
/// channel-spawned thread.
/// Returns `Ok(None)` for commits filtered out by the merge or date
/// predicates (filtering used to happen on the main thread with its
/// own `find_commit` call; both lookups are now folded here so each
/// surviving commit is parsed exactly once per worker).
fn process_commit_oid(
    oid: gix::ObjectId,
    inner: &gix::ThreadSafeRepository,
    mailmap: &gix::mailmap::Snapshot,
    include_merges: bool,
    after: Option<time::Date>,
    before: Option<time::Date>,
) -> Result<Option<CommitEvent>> {
    let repo = inner.to_thread_local();
    let commit = repo
        .find_commit(oid)
        .map_err(|e| CodeLoreError::Repo(format!("find_commit: {e}")))?;

    // R4: merge filter (mirrors GitCliRepo's `if !opts.include_merges`).
    if !include_merges && commit.parent_ids().count() > 1 {
        return Ok(None);
    }
    // R2: date-range filter on author date. `after`/`before` are
    // `time::Date` (calendar-day precision — matches git's `--after`/
    // `--before` semantics). Extract the calendar date from the full
    // `OffsetDateTime` so we stay day-precise even though the timestamp
    // is full-resolution downstream.
    if after.is_some() || before.is_some() {
        let ts = commit_author_date(&commit)?;
        let calendar_date = ts.date();
        if let Some(after) = after
            && calendar_date < after
        {
            return Ok(None);
        }
        if let Some(before) = before
            && calendar_date > before
        {
            return Ok(None);
        }
    }

    let id_string = oid.to_hex().to_string();
    let mut event = commit_event_from_gix(&commit)?;
    // Reuse the already-resolved `commit` to compute changed files —
    // avoids a redundant `find_commit` lookup flagged by the deep-analysis
    // perf review.
    event.changes = changed_files_for_commit(&repo, &commit, &id_string)?;

    // Pass the actual author_name (not b"") so
    // `.mailmap` entries of the form `Canonical <c@x> Original <o@x>`
    // — which match by Name+Email — also resolve.
    let canonical = {
        use gix::bstr::ByteSlice as _;
        let sig_ref = gix::actor::SignatureRef {
            name: event.author_name.as_bytes().as_bstr(),
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
    event.canonical_author = Some(canonical);
    let ai_attr =
        crate::identity::ai_attribution(&event.author_email, &event.author_name, &event.message);
    event.ai_attribution = Some(ai_attr.to_string());
    Ok(Some(event))
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
        .map_err(|e| CodeLoreError::Repo(format!("commit timestamp {ts_seconds}: {e}")))?;

    let committer_date = commit_committer_date(commit)?;

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
        committer_date,
        message,
        parents,
        changes: vec![],        // Populated by the caller; see walk_commits.
        canonical_author: None, // Populated by walk_commits after mailmap resolution.
        ai_attribution: None,   // Populated by walk_commits after identity classification.
        kamei: None,
    })
}

/// Iterator wrapper that joins the walker thread after the receiver
/// drains and surfaces any panic as a final `Err`. The default
/// `rx.into_iter()` shape silently swallows walker panics — `tx`
/// drops during unwind, the iterator hits end-of-stream, and the
/// caller sees a clean (but truncated) commit walk with no signal.
struct WalkerStream<I: Iterator<Item = Result<CommitEvent>>> {
    inner: I,
    handle: Option<std::thread::JoinHandle<()>>,
    /// `Some(panic_payload)` would re-yield the panic on every `next`
    /// call. We surface it ONCE then settle into `None` to match the
    /// fused-iterator contract callers expect.
    surfaced_panic: bool,
}

impl<I: Iterator<Item = Result<CommitEvent>>> Iterator for WalkerStream<I> {
    type Item = Result<CommitEvent>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(item) = self.inner.next() {
            return Some(item);
        }
        // Receiver drained. Join the walker thread so a panic surfaces
        // instead of being silently swallowed by the drop of `tx`.
        if !self.surfaced_panic
            && let Some(handle) = self.handle.take()
            && let Err(payload) = handle.join()
        {
            self.surfaced_panic = true;
            return Some(Err(CodeLoreError::Repo(
                crate::facts::ingest::format_panic_payload(&payload),
            )));
        }
        None
    }
}

#[cfg(test)]
mod walker_stream_tests {
    use super::WalkerStream;
    use crate::{CodeLoreError, CommitEvent, Result};

    /// A panic in the walker thread must surface as a final `Err(Repo)`
    /// from the iterator chain, not vanish as silent end-of-stream.
    #[test]
    fn surfaces_walker_thread_panic_as_final_err() {
        let (tx, rx) = crossbeam_channel::bounded::<Result<CommitEvent>>(4);
        let handle = std::thread::Builder::new()
            .name("test-panicking-walker".into())
            .spawn(move || {
                drop(tx);
                panic!("simulated walker explosion");
            })
            .expect("spawn");
        let mut stream = WalkerStream {
            inner: rx.into_iter(),
            handle: Some(handle),
            surfaced_panic: false,
        };
        // Receiver drains immediately (tx dropped before panic), so the
        // first `next()` joins the handle and surfaces the panic.
        let item = stream.next().expect("first item should be Some(Err)");
        let err = item.expect_err("walker panic must surface as Err");
        match err {
            CodeLoreError::Repo(msg) => {
                assert!(
                    msg.contains("commit walker thread panicked")
                        && msg.contains("simulated walker explosion"),
                    "panic message lost: {msg}"
                );
            }
            other => panic!("expected CodeLoreError::Repo, got {other:?}"),
        }
        // Fused: subsequent next() returns None.
        assert!(stream.next().is_none(), "stream must fuse after panic");
    }

    /// Happy path: the walker exits cleanly, the stream drains, and
    /// `join()` returns `Ok(())` so no spurious `Err` is yielded.
    #[test]
    fn clean_exit_does_not_yield_spurious_err() {
        let (tx, rx) = crossbeam_channel::bounded::<Result<CommitEvent>>(4);
        let handle = std::thread::Builder::new()
            .name("test-clean-walker".into())
            .spawn(move || {
                drop(tx);
            })
            .expect("spawn");
        let mut stream = WalkerStream {
            inner: rx.into_iter(),
            handle: Some(handle),
            surfaced_panic: false,
        };
        assert!(stream.next().is_none(), "clean exit must yield None");
    }
}
