//! gix-backed Repo impl. The production default.

use std::path::Path;

use gix::diff::tree_with_rewrites::Change as GixChange;
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};

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
        // F27 fix: collect OIDs WITHOUT parsing commit objects on the main
        // thread. The previous filter pass called `repo.find_commit(oid)`
        // for every reachable commit on the hot path, then `process_commit_oid`
        // called `find_commit` AGAIN on the worker — two object-store lookups
        // per surviving commit, with the first one serialised on a single
        // thread. Filtering is now folded into `process_commit_oid`
        // (returning `Result<Option<CommitEvent>>`), so the OID gather is
        // pure index iteration and filtering parallelises across workers.
        //
        // F12 invariant (commits.rowid ASC = gix walk order) is preserved:
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

        // F13 fix: stream events through a bounded crossbeam channel
        // rather than eagerly collecting the full event list into memory.
        // The previous F9 implementation called `par_iter().collect()`
        // which materialised gigabytes on large repos (100k+ commits with
        // rich changes/hunks per commit), bypassed the producer-consumer
        // channel architecture, and could OOM CI runners.
        //
        // The architectural challenge: F12's `commits.rowid ASC` tiebreak
        // REQUIRES insertion order to match commit-walk order. Pure
        // streaming (`par_iter().for_each(send)`) scrambles order across
        // worker threads and silently breaks F12.
        //
        // Resolution: chunked rayon. Process oids in batches of
        // `WALKER_CHUNK_SIZE`, each batch parallelised with
        // order-preserving `collect::<Vec<_>>`, then drained serially
        // through the channel. Order is preserved both within and across
        // chunks (chunks processed sequentially in the driver thread).
        // Peak memory: one chunk's events (~1 MB at 1000 × typical event
        // size) + channel buffer. Bounded regardless of repo size.
        #[allow(clippy::items_after_statements)]
        const WALKER_CHUNK_SIZE: usize = 1000;
        #[allow(clippy::items_after_statements)]
        const WALKER_CHANNEL_CAPACITY: usize = 256;

        let inner_clone = self.inner.clone();
        // Parse `.mailmap` ONCE up front; the snapshot is owned bytes
        // (Send + Sync) and is shared across all workers.
        let mailmap = inner_clone.to_thread_local().open_mailmap();
        let (tx, rx) = crossbeam_channel::bounded::<Result<CommitEvent>>(WALKER_CHANNEL_CAPACITY);

        std::thread::Builder::new()
            .name("codelore-gix-walker".into())
            .spawn(move || {
                for chunk in oids.chunks(WALKER_CHUNK_SIZE) {
                    // Order-preserving parallel map over this chunk. Filter
                    // logic moved INSIDE the worker (F27): each worker opens
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

        // rx.into_iter() is `Iterator<Item = Result<CommitEvent>> + Send + 'static`,
        // which satisfies the trait's `+ 'a` bound for any `'a`.
        Ok(Box::new(rx.into_iter()))
    }

    fn changed_files(&self, rev: &str) -> Result<Vec<FileChange>> {
        compute_changed_files(&self.inner, rev)
    }

    fn diff_hunks(&self, _rev: &str, _path: &str) -> Result<Vec<Hunk>> {
        Ok(vec![]) // Plan 4 lands real hunk extraction
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

    fn commit_metadata(&self, rev: &str) -> Result<CommitMetadata> {
        Ok(CommitMetadata {
            rev: rev.to_string(),
            signed: false,
            signed_by: None,
            signoffs: vec![],
        })
    }

    fn head_sha(&self) -> Result<String> {
        let repo = self.inner.to_thread_local();
        let oid = repo
            .head_id()
            .map_err(|e| CodeLoreError::Repo(format!("head_id: {e}")))?;
        Ok(oid.to_hex().to_string())
    }

    fn is_worktree_dirty(&self) -> bool {
        // gix's `Repository::status(progress)` returns a `Platform` whose
        // `into_iter()` yields a unified stream covering:
        //   1. Tracked-file modifications (index vs worktree),
        //   2. Untracked files (via the dirwalk),
        //   3. Staged-vs-HEAD differences.
        //
        // F11 fix: previously we used `into_index_worktree_iter` which
        // ONLY yields (1) — it SKIPS the dirwalk and therefore reports
        // untracked-only repos as clean. `GitCliRepo` (via
        // `git status --porcelain`) DOES report untracked files. The
        // backends diverged. Switching to the full `into_iter()` brings
        // them into agreement.
        //
        // Errors are deliberately swallowed (`return false` on any failure):
        // detection is a hint that triggers a tracing::warn!, not a
        // contract. A missed warning is strictly preferable to a hard
        // analyze failure on a status-API edge case.
        let repo = self.inner.to_thread_local();
        let Ok(platform) = repo.status(gix::progress::Discard) else {
            return false;
        };
        let Ok(iter) = platform.into_iter(Vec::new()) else {
            return false;
        };
        // Any single yielded item — modified, untracked, or staged — means
        // the tree is dirty.
        iter.flatten().next().is_some()
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
            let (loc_added, loc_deleted) = count_loc(repo, Some(previous_id), Some(id))?;
            Ok(Some(FileChange {
                path: location.to_string(),
                change_type: ChangeType::Modified,
                loc_added,
                loc_deleted,
                hunks: vec![],
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

/// F34 fix: blobs larger than [`MAX_DIFF_BLOB_BYTES`] on either side, or
/// containing a NUL byte in the first [`BINARY_SNIFF_BYTES`], return
/// `(0, 0)` without ever loading the full bytes into `InternedInput` or
/// running the histogram diff. Pre-F34, `count_loc` blindly read raw
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
    use gix::diff::blob::{Algorithm, InternedInput, diff_with_slider_heuristics};

    let empty: Vec<u8> = Vec::new();
    let read_blob = |oid: gix::ObjectId| -> Result<Vec<u8>> {
        let obj = repo.find_object(oid).map_err(|_e| {
            // Distinguish "object missing" from other repo errors so a
            // shallow / corrupted repo can be detected upstream.
            CodeLoreError::BlobNotFound {
                oid: oid.to_string(),
            }
        })?;
        Ok(obj.data.clone())
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
        return Ok((0, 0));
    }

    let input = InternedInput::new(old_bytes.as_slice(), new_bytes.as_slice());
    let diff = diff_with_slider_heuristics(Algorithm::Histogram, &input);
    #[allow(clippy::cast_possible_truncation)]
    let added = diff.count_additions() as u32;
    #[allow(clippy::cast_possible_truncation)]
    let removed = diff.count_removals() as u32;
    Ok((added, removed))
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

/// F13 helper: extract a fully-resolved `CommitEvent` from a single oid.
/// Called by every rayon worker in the chunked walker — each worker
/// constructs its own thread-local `gix::Repository` from the shared
/// `ThreadSafeRepository` clone, finds the commit, computes changes,
/// resolves mailmap canonical author, and classifies AI attribution.
///
/// Free function (not a closure) so the chunked rayon driver can call it
/// directly without dragging closure-capture lifetimes through the
/// channel-spawned thread.
/// Returns `Ok(None)` for commits filtered out by the merge or date
/// predicates (F27: filtering used to happen on the main thread with its
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

    // Plan 8 §2 T6 finding: pass the actual author_name (not b"") so
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
