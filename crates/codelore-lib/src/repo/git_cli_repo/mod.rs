//! Shell-out impl of the `Repo` trait — treats C git as ground truth.
//! Used in the differential-testing harness to validate that `GixRepo`'s
//! gitoxide-based reads produce the same output as canonical C git.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::repo::Repo;
use crate::{CodeLoreError, CommitEvent, FileChange, Hunk, Options, Result};

mod history;
mod tree;
mod worktree;

use history::{parse_changes_block, parse_git_log_stream, parse_hunk_headers};
use tree::parse_ls_tree_record;
use worktree::parse_porcelain_v2;

pub struct GitCliRepo {
    root: PathBuf,
}

impl GitCliRepo {
    pub fn open(root: &Path) -> Result<Self> {
        let output = Command::new("git")
            // Disable path-quoting so non-ASCII or space-containing paths
            // come back as raw UTF-8 bytes from `rev-parse` too. Git's
            // default `core.quotepath=true` wraps such paths in `"..."`
            // and octal-escapes non-ASCII bytes — fine for git's own UI,
            // wrong for parity with `GixRepo` which reads raw bytes from
            // the object database. Matters here only for symmetry with
            // `run_git`; this call doesn't actually emit paths.
            .args(["-c", "core.quotepath=false", "rev-parse", "--git-dir"])
            .current_dir(root)
            .stdin(Stdio::null())
            .output()
            .map_err(|e| CodeLoreError::Repo(format!("git rev-parse: {e}")))?;
        if !output.status.success() {
            return Err(CodeLoreError::Repo(format!(
                "not a git repo: {}",
                root.display()
            )));
        }
        Ok(Self {
            root: root.to_path_buf(),
        })
    }

    fn run_git(&self, args: &[&str]) -> Result<std::process::Output> {
        // `core.quotepath=false` injected on every invocation so non-ASCII
        // / space-containing paths come back as raw UTF-8, matching the
        // raw-byte paths that `GixRepo` returns from gitoxide's object DB.
        // Without this, `git log --raw` for a path like `café.rs` would
        // emit `"caf\303\251.rs"` (quoted, octal-escaped) while `GixRepo`
        // emits `café.rs` — splitting per-file aggregations (hotspots,
        // churn, ownership) silently across two key values for the same
        // physical file, and breaking cross-walker differential parity.
        let mut full_args: Vec<&str> = Vec::with_capacity(args.len() + 2);
        full_args.push("-c");
        full_args.push("core.quotepath=false");
        full_args.extend_from_slice(args);
        Command::new("git")
            .args(&full_args)
            .current_dir(&self.root)
            .stdin(Stdio::null())
            .output()
            .map_err(|e| CodeLoreError::Repo(format!("git {}: {e}", args.join(" "))))
    }

    /// Resolve `marker` to its worktree-correct location via `git rev-parse
    /// --git-path` and report whether that path exists on disk. `rev-parse`
    /// prints an absolute path when the git dir lies outside the worktree
    /// (linked worktrees) and a relative one otherwise; `self.root.join`
    /// handles both (an absolute join replaces the base). Returns `false` on
    /// any git or UTF-8 failure — a missed hint, never a hard error.
    fn git_path_exists(&self, marker: &str) -> bool {
        let Ok(output) = self.run_git(&["rev-parse", "--git-path", marker]) else {
            return false;
        };
        if !output.status.success() {
            return false;
        }
        let Ok(resolved) = String::from_utf8(output.stdout) else {
            return false;
        };
        let resolved = resolved.trim();
        if resolved.is_empty() {
            return false;
        }
        self.root.join(resolved).exists()
    }
}

impl Repo for GitCliRepo {
    fn walk_commits<'a>(
        &'a self,
        opts: &'a Options,
    ) -> Result<Box<dyn Iterator<Item = Result<CommitEvent>> + Send + 'a>> {
        // Format: US (0x1f) separates fields within a record; RS (0x1e) terminates the
        // pretty-format block for each commit. The name-status lines follow immediately
        // after the RS in the same output stream.
        // Fields: SHA, parents (space-separated), author_email, author_name,
        //         committer_email, author ISO date, full message body.
        let mut args = vec![
            "log",
            "--pretty=format:%H%x1f%P%x1f%ae%x1f%an%x1f%ce%x1f%aI%x1f%cI%x1f%B%x1e",
            // `--raw --numstat` together produce a per-commit block of raw
            // lines (status + paths, `:`-prefixed) immediately followed by
            // numstat lines (added/deleted/path). They appear in matching
            // file order so we can zip them by index. We need both because
            // `--numstat` alone can't distinguish Added from Modified
            // (zero-delete numstat looks the same), and `--name-status`
            // alone has no line counts.
            "--raw",
            "--numstat",
        ];
        if !opts.include_merges {
            args.push("--no-merges");
        }
        // date filtering
        let after_str;
        let before_str;
        if let Some(after) = opts.after {
            after_str = format!("--after={after}");
            args.push(&after_str);
        }
        if let Some(before) = opts.before {
            before_str = format!("--before={before}");
            args.push(&before_str);
        }

        let output = self.run_git(&args)?;
        if !output.status.success() {
            return Err(CodeLoreError::Repo(format!(
                "git log failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        let raw = String::from_utf8(output.stdout)
            .map_err(|e| CodeLoreError::Repo(format!("git log output not utf-8: {e}")))?;

        let mut events = parse_git_log_stream(&raw);

        // Walk-time mailmap resolution. Matches GixRepo's gix-mailmap pass
        // so the two walkers produce identical `canonical_author` columns
        // on the same fixture — the differential parity tests depend on
        // it. Cache by (name, email) pair so we only spawn `git check-mailmap`
        // once per unique identity (N events, M ≤ N unique identities).
        // The cache key includes name because `.mailmap` name+email rules
        // mean a single email can resolve differently for different names.
        let mut mailmap_cache: std::collections::HashMap<(String, String), String> =
            std::collections::HashMap::new();
        for event in &mut events {
            // Cache key INCLUDES author_name now — a single email can resolve
            // differently depending on the name it ships with (when the
            // `.mailmap` uses the name+email rule form). Sharing a single
            // cache entry across all (name, email) pairs that share an email
            // would silently apply one name's resolution to all of them.
            let cache_key = (event.author_name.clone(), event.author_email.clone());
            let canonical = mailmap_cache
                .entry(cache_key)
                .or_insert_with(|| self.resolve_alias(&event.author_name, &event.author_email))
                .clone();
            // Only flag a canonical when it actually differs from the raw
            // email — keeping `None` for non-aliased authors mirrors gix's
            // behaviour and avoids polluting cache keys with the noop case.
            if canonical != event.author_email {
                event.canonical_author = Some(canonical);
            }
        }

        let events: Vec<Result<CommitEvent>> = events.into_iter().map(Ok).collect();
        Ok(Box::new(events.into_iter()))
    }

    fn changed_files(&self, rev: &str) -> Result<Vec<FileChange>> {
        // `git show --raw --numstat --pretty=format:` emits the same per-commit
        // raw + numstat block our streaming `git log` consumer parses,
        // minus the pretty header. `parse_changes_block` accepts both
        // (it just ignores empty lines and pairs raw with numstat by order).
        let output = self.run_git(&["show", "--raw", "--numstat", "--pretty=format:", rev])?;
        if !output.status.success() {
            return Err(CodeLoreError::Repo(format!(
                "git show: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }
        let raw = String::from_utf8(output.stdout)
            .map_err(|e| CodeLoreError::Repo(format!("git show output not utf-8: {e}")))?;
        Ok(parse_changes_block(&raw))
    }

    fn diff_hunks(&self, rev: &str, path: &str) -> Result<Vec<Hunk>> {
        // `git show --format= -p --unified=0` handles root commits correctly because
        // it diffs against the empty tree. `git diff <rev>^..<rev>` fails for root commits.
        let output = self.run_git(&["show", "--format=", "-p", "--unified=0", rev, "--", path])?;
        if !output.status.success() {
            return Err(CodeLoreError::Repo(format!(
                "git show -p: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }
        let raw = String::from_utf8(output.stdout)
            .map_err(|e| CodeLoreError::Repo(format!("git show -p output not utf-8: {e}")))?;
        Ok(parse_hunk_headers(&raw))
    }

    fn resolve_alias(&self, name: &str, email: &str) -> String {
        // `git check-mailmap` accepts either `<email>` (email-only match) or
        // `Name <email>` (name+email match). Passing the full identity lets
        // `.mailmap` rules of the form
        //
        //     Canonical Name <canonical@email> Old Name <old@email>
        //
        // resolve correctly. Earlier this method passed only `<{email}>`,
        // matching email-only rules but silently missing name+email rules
        // — diverging from `GixRepo::walk_commits`'s inline resolution and
        // breaking cross-walker parity on any `.mailmap` that used the
        // 4-token form. If `name` is empty, fall back to the `<email>`
        // form (matches git's own behaviour for nameless contacts).
        let arg = if name.is_empty() {
            format!("<{email}>")
        } else {
            format!("{name} <{email}>")
        };
        // `core.quotepath=false` here is informational (check-mailmap output
        // is identity strings, not paths) — but injected for consistency
        // with `run_git` so any future change to this command's output
        // shape gets the same treatment without us forgetting.
        let Ok(output) = Command::new("git")
            .args(["-c", "core.quotepath=false", "check-mailmap", &arg])
            .current_dir(&self.root)
            .stdin(Stdio::null())
            .output()
        else {
            return email.to_string();
        };
        if !output.status.success() {
            return email.to_string();
        }
        let s = String::from_utf8_lossy(&output.stdout);
        parse_email_from_mailmap_line(s.trim()).unwrap_or_else(|| email.to_string())
    }

    fn head_sha(&self) -> Result<String> {
        let output = self.run_git(&["rev-parse", "HEAD"])?;
        if !output.status.success() {
            return Err(CodeLoreError::Repo(format!(
                "git rev-parse HEAD: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }
        let sha = String::from_utf8(output.stdout)
            .map_err(|e| CodeLoreError::Repo(format!("git rev-parse HEAD not utf-8: {e}")))?
            .trim()
            .to_string();
        Ok(sha)
    }

    fn is_worktree_dirty(&self) -> bool {
        // `git status --porcelain --untracked-files=no` emits one
        // short-format line per staged or unstaged change to a tracked
        // path, zero bytes for a clean tree; `--untracked-files=no` drops
        // the `??` lines so untracked files never count. Every caller (the
        // `calibrate-defects` mining guard, the cache-hit staleness
        // warning, the dirty cache-write skip) protects HEAD-time metrics
        // computed over `tracked_paths_at_head()` only, so untracked files
        // must not count. Errors swallowed per the trait's contract
        // (`false` on detection failure is preferable to a hard analyze
        // error).
        match self.run_git(&["status", "--porcelain", "--untracked-files=no"]) {
            Ok(output) if output.status.success() => !output.stdout.is_empty(),
            _ => false,
        }
    }

    fn is_shallow(&self) -> bool {
        // `git rev-parse --is-shallow-repository` prints `true`/`false` and
        // handles linked worktrees (where the `shallow` grafts file lives in
        // the shared common dir, not this checkout's `.git`). Mirrors
        // `GixRepo::is_shallow` so the differential suite can hold the two
        // backends to one answer; errors swallowed per the trait's
        // hint-not-contract convention (`false` on detection failure).
        match self.run_git(&["rev-parse", "--is-shallow-repository"]) {
            Ok(output) if output.status.success() => {
                String::from_utf8_lossy(&output.stdout).trim() == "true"
            }
            _ => false,
        }
    }

    fn merge_or_rebase_in_progress(&self) -> bool {
        // `MERGE_HEAD`, `CHERRY_PICK_HEAD`, and `REVERT_HEAD` are files;
        // `rebase-merge` and `rebase-apply` are directories — `Path::exists`
        // covers both. We resolve each marker's location with `git rev-parse
        // --git-path` rather than hand-joining `root/.git/<marker>`: a linked
        // worktree keeps merge/rebase state in its own git dir, not the
        // common one, and `--git-path` returns the worktree-correct path.
        // This is the same five-marker set `GixRepo`'s `state()` covers;
        // `BISECT_LOG` is deliberately not probed. A detection failure yields
        // `false` (a missed hint, per the trait contract).
        const MARKERS: [&str; 5] = [
            "MERGE_HEAD",
            "CHERRY_PICK_HEAD",
            "REVERT_HEAD",
            "rebase-merge",
            "rebase-apply",
        ];
        MARKERS.iter().any(|marker| self.git_path_exists(marker))
    }

    fn read_blob_at(&self, rev: &str, path: &str) -> Result<Option<Vec<u8>>> {
        // `git cat-file blob <rev>:<path>` resolves the path through that
        // rev's tree and writes the blob's raw bytes to stdout. Matches
        // GixRepo's ODB-backed read semantics (bare-repo safe, no
        // working-tree dependency). The `blob` type filter is load-bearing
        // for parity: it errors (→ `Ok(None)`) when the spec resolves to a
        // non-blob — a directory or a submodule gitlink — exactly where
        // GixRepo returns `Ok(None)` via its `entry.mode().is_blob()`
        // guard. `git show <rev>:<dir>` would instead succeed and print a
        // tree listing, silently diverging from GixRepo. Missing paths
        // also error → `Ok(None)`, per the trait contract.
        //
        // `cat-file` emits the object verbatim (never textconv-smudged),
        // so the bytes match GixRepo's raw ODB read.
        let output = self.run_git(&["cat-file", "blob", &format!("{rev}:{path}")])?;
        if output.status.success() {
            Ok(Some(output.stdout))
        } else {
            Ok(None)
        }
    }

    fn worktree_changes(&self) -> Result<Vec<super::WorktreeChange>> {
        // `--porcelain=v2 -z` gives NUL-framed records with per-side octal
        // modes (needed for the symlink/gitlink filter) and explicit
        // rename records; `--untracked-files=no` drops `?` records so
        // untracked files never become candidates — the same shape
        // `GixRepo` gets from its status iterator with the dirwalk off.
        let output = self.run_git(&["status", "--porcelain=v2", "-z", "--untracked-files=no"])?;
        if !output.status.success() {
            return Err(CodeLoreError::Repo(format!(
                "git status --porcelain=v2: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }
        let entries = parse_porcelain_v2(&output.stdout)?;
        if entries.is_empty() {
            return Ok(Vec::new());
        }
        // Porcelain paths are relative to the worktree top level, which can
        // differ from `self.root` when the repo was opened from a
        // subdirectory.
        let toplevel = self.run_git(&["rev-parse", "--show-toplevel"])?;
        if !toplevel.status.success() {
            return Err(CodeLoreError::Repo(format!(
                "git rev-parse --show-toplevel: {}",
                String::from_utf8_lossy(&toplevel.stderr)
            )));
        }
        let toplevel = String::from_utf8(toplevel.stdout)
            .map_err(|e| CodeLoreError::Repo(format!("worktree top level not utf-8: {e}")))?;
        let worktree_root = std::path::PathBuf::from(toplevel.trim_end());

        let mut candidates = std::collections::BTreeMap::new();
        for entry in entries {
            super::add_worktree_candidate(&mut candidates, entry.path, entry.rename_from);
        }
        super::net_classify_candidates(self, &worktree_root, candidates)
    }

    fn tracked_paths_at_head(&self) -> Result<Vec<String>> {
        // `ls-tree -r -z` walks HEAD's tree recursively; with `-z`, git
        // emits paths verbatim (no quoting), so spaces and newlines
        // survive intact regardless of `core.quotepath` — `run_git`'s
        // injected `core.quotepath=false` is belt-and-suspenders for the
        // non-`-z` invocations it also serves. Record shape:
        // `<mode> <type> <oid>\t<path>`.
        let output = self.run_git(&["ls-tree", "-r", "-z", "HEAD"])?;
        if !output.status.success() {
            return Err(CodeLoreError::Repo(format!(
                "git ls-tree: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }
        let mut paths: Vec<String> = output
            .stdout
            .split(|b| *b == 0)
            .filter_map(parse_ls_tree_record)
            .collect();
        // ls-tree emits git tree order (directories sort with a virtual
        // trailing `/`); sort explicitly to match GixRepo's deterministic
        // ascending order over full paths.
        paths.sort_unstable();
        Ok(paths)
    }

    fn tags(&self) -> Result<Vec<super::TagInfo>> {
        use super::TagInfo;

        // NUL-delimited fields per ref: short-name, objectname, peeled-objectname,
        // taggerdate (iso-strict, annotated only), committerdate (iso-strict, lightweight only).
        // `%(*objectname)` is non-empty only for annotated tags; it holds the
        // peeled commit OID that the tag object points at. `run_git` prepends
        // `-c core.quotepath=false` automatically.
        let output = self.run_git(&[
            "for-each-ref",
            "refs/tags",
            "--format=%(refname:short)%00%(objectname)%00%(*objectname)%00%(taggerdate:iso-strict)%00%(committerdate:iso-strict)",
        ])?;
        if !output.status.success() {
            return Err(CodeLoreError::Repo(format!(
                "git for-each-ref refs/tags: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }
        let stdout = String::from_utf8(output.stdout)
            .map_err(|e| CodeLoreError::Repo(format!("for-each-ref output: {e}")))?;

        let mut tags = Vec::new();
        for line in stdout.lines() {
            if line.is_empty() {
                continue;
            }
            let fields: Vec<&str> = line.splitn(5, '\0').collect();
            if fields.len() < 5 {
                continue;
            }
            let name = fields[0].to_string();
            let objectname = fields[1]; // direct tag ref OID
            let peeled = fields[2]; // commit OID for annotated tags; "" for lightweight
            let taggerdate = fields[3]; // non-empty for annotated tags
            let committerdate = fields[4]; // non-empty for lightweight tags

            // Non-empty `peeled` means this is an annotated tag: the tag object's
            // tagger date is the semantically correct sort key.
            let (target_rev, date_str) = if peeled.is_empty() {
                // Lightweight tag: ref points directly to a commit.
                (objectname.to_string(), committerdate)
            } else {
                // Annotated tag: `peeled` is the commit OID; use the tagger date.
                (peeled.to_string(), taggerdate)
            };

            let date = parse_iso_timestamp(date_str).ok_or_else(|| {
                CodeLoreError::Repo(format!("could not parse date {date_str:?} for tag {name}"))
            })?;

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

fn parse_email_from_mailmap_line(line: &str) -> Option<String> {
    let (_, after) = line.rsplit_once('<')?;
    let (email, _) = after.rsplit_once('>')?;
    Some(email.to_string())
}

/// Parse `git log %aI` output (ISO 8601 with offset, e.g.
/// `"2026-06-06T19:29:04+02:00"`) into a full `OffsetDateTime`.
///
/// The `time` crate is compiled without the `parsing` feature in this
/// workspace (see `codelore-lib/Cargo.toml`), so we hand-parse instead of
/// using `OffsetDateTime::parse`. The shape `git log --pretty=%aI`
/// emits is fixed and ASCII-only, so byte slicing is safe.
///
/// Returns `None` on any malformed input — caller drops the commit.
fn parse_iso_timestamp(s: &str) -> Option<time::OffsetDateTime> {
    use time::{Date, Month, PrimitiveDateTime, Time, UtcOffset};
    let s = s.trim();
    // Minimum: `YYYY-MM-DDTHH:MM:SSZ` = 20 chars. We also accept `±HH:MM`
    // offsets (25 chars).
    if s.len() < 20 {
        return None;
    }
    let year: i32 = s[0..4].parse().ok()?;
    let month: u8 = s[5..7].parse().ok()?;
    let day: u8 = s[8..10].parse().ok()?;
    let hour: u8 = s[11..13].parse().ok()?;
    let minute: u8 = s[14..16].parse().ok()?;
    let second: u8 = s[17..19].parse().ok()?;
    let month = Month::try_from(month).ok()?;
    let date = Date::from_calendar_date(year, month, day).ok()?;
    let time = Time::from_hms(hour, minute, second).ok()?;
    let primitive = PrimitiveDateTime::new(date, time);

    // Offset: `Z` (UTC) or `±HH:MM` starting at byte 19.
    let offset = match s.as_bytes().get(19)? {
        b'Z' => UtcOffset::UTC,
        b'+' | b'-' => {
            // Need at least `±HH:MM` (6 chars from offset start).
            if s.len() < 25 {
                return None;
            }
            let sign: i8 = if s.as_bytes()[19] == b'+' { 1 } else { -1 };
            let off_hour: i8 = s[20..22].parse().ok()?;
            let off_min: i8 = s[23..25].parse().ok()?;
            UtcOffset::from_hms(sign * off_hour, sign * off_min, 0).ok()?
        }
        _ => return None,
    };
    Some(primitive.assume_offset(offset))
}
