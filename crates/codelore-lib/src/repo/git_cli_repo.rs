//! Shell-out impl of the `Repo` trait — treats C git as ground truth.
//! Used in Plan 6's differential-testing harness to validate that `GixRepo`'s
//! gitoxide-based reads produce the same output as canonical C git.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use time::Date;

use crate::repo::{CommitMetadata, Repo};
use crate::{ChangeType, CodeLoreError, CommitEvent, FileChange, Hunk, Options, Result};

pub struct GitCliRepo {
    root: PathBuf,
}

impl GitCliRepo {
    pub fn open(root: &Path) -> Result<Self> {
        let output = Command::new("git")
            .args(["rev-parse", "--git-dir"])
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
        Command::new("git")
            .args(args)
            .current_dir(&self.root)
            .stdin(Stdio::null())
            .output()
            .map_err(|e| CodeLoreError::Repo(format!("git {}: {e}", args.join(" "))))
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
            "--pretty=format:%H%x1f%P%x1f%ae%x1f%an%x1f%ce%x1f%aI%x1f%B%x1e",
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
        // it. Cache by raw email so we only spawn `git check-mailmap` once
        // per unique author (N events, M ≤ N unique authors).
        let mut mailmap_cache: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        for event in &mut events {
            let canonical = mailmap_cache
                .entry(event.author_email.clone())
                .or_insert_with(|| self.resolve_alias(&event.author_email))
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

    fn resolve_alias(&self, email: &str) -> String {
        let arg = format!("<{email}>");
        let Ok(output) = Command::new("git")
            .args(["check-mailmap", &arg])
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

    fn commit_metadata(&self, rev: &str) -> Result<CommitMetadata> {
        // Signed status: %G? returns "G"=good, "B"=bad, "U"=unknown, "N"=no sig, "E"=error
        // Signer key/name: %GS (signer name)
        // Trailers: %(trailers:key=Signed-off-by,valueonly) — one per line
        let output = self.run_git(&[
            "show",
            "--no-patch",
            "--format=%G?%x1f%GS%x1f%(trailers:key=Signed-off-by,valueonly)",
            rev,
        ])?;
        if !output.status.success() {
            return Err(CodeLoreError::Repo(format!(
                "git show (metadata): {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }
        let raw = String::from_utf8(output.stdout)
            .map_err(|e| CodeLoreError::Repo(format!("git show metadata not utf-8: {e}")))?;

        let raw = raw.trim();
        // The format emits one line: "<gpg_flag>\x1f<signer>\x1f<trailers...>"
        // but trailers may be empty or span multiple lines.
        // Split on the first two \x1f separators.
        let mut parts = raw.splitn(3, '\x1f');
        let gpg_flag = parts.next().unwrap_or("N");
        let signer_raw = parts.next().unwrap_or("").trim();
        let trailers_block = parts.next().unwrap_or("");

        let signed = matches!(gpg_flag, "G" | "U" | "X" | "Y");
        let signed_by = if signed && !signer_raw.is_empty() {
            Some(signer_raw.to_string())
        } else {
            None
        };

        let signoffs: Vec<String> = trailers_block
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect();

        Ok(CommitMetadata {
            rev: rev.to_string(),
            signed,
            signed_by,
            signoffs,
        })
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
}

// ---------------------------------------------------------------------------
// Parser helpers
// ---------------------------------------------------------------------------

fn parse_email_from_mailmap_line(line: &str) -> Option<String> {
    let (_, after) = line.rsplit_once('<')?;
    let (email, _) = after.rsplit_once('>')?;
    Some(email.to_string())
}

/// Parse the output of:
///   `git log --pretty=format:"%H%x1f%P%x1f%ae%x1f%an%x1f%ce%x1f%aI%x1f%B%x1e" --name-status`
///
/// The RS (`\x1e`) character separates the pretty-format block from the
/// name-status block, but crucially the name-status for commit K appears in
/// the chunk AFTER the RS that ends commit K's pretty block — not before it.
///
/// For N commits the output looks like:
/// ```text
///   pretty0 \x1e \n name-status0 \n\n pretty1 \x1e \n name-status1 \n\n ... prettyN-1 \x1e \n name-statusN-1 \n
/// ```
///
/// When we split on `\x1e` we get N+1 chunks:
/// - chunk[0]     : pretty0
/// - chunk[k]     : `\n` name-status_{k-1} `\n\n` `pretty_k`   (for 1 ≤ k ≤ N-1)
/// - chunk[N]     : `\n` name-status_{N-1}
///
/// So each commit's pretty block is in chunk[k] and its name-status is in chunk[k+1].
fn parse_git_log_stream(raw: &str) -> Vec<CommitEvent> {
    let chunks: Vec<&str> = raw.split('\x1e').collect();
    if chunks.is_empty() {
        return vec![];
    }

    let n = chunks.len(); // N+1 chunks for N commits
    let mut events = Vec::with_capacity(n.saturating_sub(1));

    for (k, chunk) in chunks.iter().enumerate() {
        // The last chunk contains only name-status for the previous commit; no pretty block.
        // It has already been consumed when we processed chunk[N-1].
        if k == n - 1 {
            break;
        }

        // Extract the pretty block for commit k.
        // chunk[0] is purely pretty; chunk[k ≥ 1] starts with "\n<name-status>\n\n<pretty>".
        let pretty = if k == 0 {
            chunk
        } else {
            // Skip the leading "\n<name-status>\n\n" prefix to reach the pretty block.
            split_off_name_status_prefix(chunk)
        };

        let pretty = pretty.trim_start_matches('\n');
        if pretty.is_empty() {
            continue;
        }

        // The name-status for commit k is in chunks[k+1], before the next pretty block.
        let name_status_chunk = chunks[k + 1];
        let name_status = extract_name_status_prefix(name_status_chunk);

        if let Some(event) = parse_pretty_block(pretty, name_status) {
            events.push(event);
        }
    }

    events
}

/// Returns `true` if the first non-empty line of `s` looks like a pretty-format block
/// (i.e. it contains a US `\x1f` field separator), meaning there is no name-status prefix.
///
/// This is used to detect the case where a merge commit has an empty name-status block:
/// the chunk after the merge's `\x1e` starts directly with the next commit's pretty block
/// instead of the usual `\n<name-status>\n\n<pretty>` structure.
fn starts_with_pretty_block(s: &str) -> bool {
    s.trim_start_matches('\n')
        .lines()
        .next()
        .is_some_and(|l| l.contains('\x1f'))
}

/// From a chunk that starts with `\n<name-status>\n\n<pretty>`, return the pretty portion.
///
/// Special case: if the chunk starts directly with a pretty block (no name-status lines,
/// no `\n\n` separator — as happens after a merge commit with empty name-status), return
/// the entire chunk so the commit is not silently dropped.
fn split_off_name_status_prefix(chunk: &str) -> &str {
    // Fast path: if the chunk starts with a pretty block, there is no name-status prefix.
    if starts_with_pretty_block(chunk) {
        return chunk;
    }
    // Find the blank line ("\n\n") that separates name-status from the next pretty block.
    // The chunk starts with '\n', then name-status lines, then '\n\n', then the pretty block.
    if let Some(pos) = find_double_newline(chunk) {
        &chunk[pos + 2..]
    } else {
        // No blank line separator found — entire chunk is name-status, no pretty block.
        ""
    }
}

/// From a chunk that starts with `\n<name-status>\n\n<pretty or end>`, extract just the name-status.
///
/// Special case: if the chunk starts directly with a pretty block (no name-status), return
/// an empty string so the previous commit gets an empty (correct) name-status for a merge.
fn extract_name_status_prefix(chunk: &str) -> &str {
    // If the chunk is actually a pretty block (merge with empty name-status case),
    // the "name-status" for the previous commit is empty.
    if starts_with_pretty_block(chunk) {
        return "";
    }
    let chunk = chunk.trim_start_matches('\n');
    if let Some(pos) = find_double_newline(chunk) {
        &chunk[..pos]
    } else {
        // Last chunk: entire content (after leading newline) is name-status.
        chunk.trim_end_matches('\n')
    }
}

/// Find the position of the first `\n\n` sequence in `s`.
fn find_double_newline(s: &str) -> Option<usize> {
    s.as_bytes().windows(2).position(|w| w == b"\n\n")
}

fn parse_pretty_block(pretty: &str, name_status: &str) -> Option<CommitEvent> {
    // pretty is: SHA\x1fPARENTS\x1fAE\x1fAN\x1fCE\x1fAISO\x1fMESSAGE\n
    let mut parts = pretty.splitn(7, '\x1f');
    let sha = parts.next()?.trim().to_string();
    if sha.is_empty() || sha.len() < 7 {
        return None;
    }
    let parents_raw = parts.next()?.trim().to_string();
    let author_email = parts.next()?.trim().to_string();
    let author_name = parts.next()?.trim().to_string();
    let committer_email = parts.next()?.trim().to_string();
    let date_str = parts.next()?.trim().to_string();
    let message = parts
        .next()
        .unwrap_or("")
        .trim_end_matches('\n')
        .to_string();

    let parents: Vec<String> = if parents_raw.is_empty() {
        vec![]
    } else {
        parents_raw.split_whitespace().map(str::to_string).collect()
    };

    let date = parse_iso_date(&date_str)?;
    let changes = parse_changes_block(name_status);

    Some(CommitEvent {
        rev: sha,
        author_email,
        author_name,
        committer_email,
        date,
        message,
        parents,
        changes,
        canonical_author: None, // Filled in by walk_commits' mailmap-cache pass.
        ai_attribution: None,   // Re-classified in ingest_loop with .codelorebots patterns.
        kamei: None,
    })
}

/// Parse a per-commit raw + numstat block:
///
/// ```text
/// :100644 100644 d00491f 2b2f2e1 M\tsrc/main.rs
/// :100644 100644 0cfbf08 0cfbf08 R100\tsrc/old.rs\tsrc/new.rs
/// 1\t0\tsrc/main.rs
/// 0\t0\tsrc/old.rs => src/new.rs
/// ```
///
/// Raw lines (`:`-prefixed) and numstat lines appear in matching file
/// order; we zip them by index.
fn parse_changes_block(block: &str) -> Vec<FileChange> {
    let mut raw_entries: Vec<&str> = Vec::new();
    let mut numstat_entries: Vec<&str> = Vec::new();
    for line in block.lines() {
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with(':') {
            raw_entries.push(trimmed);
        } else {
            numstat_entries.push(trimmed);
        }
    }
    raw_entries
        .into_iter()
        .zip(numstat_entries)
        .filter_map(|(raw, numstat)| parse_raw_numstat_pair(raw, numstat))
        .collect()
}

/// Parse a single raw line paired with its numstat line.
///
/// Raw format: `:<mode1> <mode2> <hash1> <hash2> <STATUS>\t<path>[\t<path2>]`
/// Numstat format: `<added>\t<deleted>\t<path>` — or `<old> => <new>` for renames.
fn parse_raw_numstat_pair(raw: &str, numstat: &str) -> Option<FileChange> {
    let raw = raw.strip_prefix(':')?;
    // The header (`mode mode hash hash STATUS`) is space-separated; paths
    // follow a tab. Status sits in the last whitespace-separated field
    // before the tab boundary.
    let tab = raw.find('\t')?;
    let header = &raw[..tab];
    let paths_part = &raw[tab + 1..];
    let mut header_iter = header.split_whitespace();
    let _mode_src = header_iter.next()?;
    let _mode_dst = header_iter.next()?;
    let _hash_src = header_iter.next()?;
    let _hash_dst = header_iter.next()?;
    let status = header_iter.next()?;

    let mut paths = paths_part.split('\t');
    let path1 = paths.next()?.to_string();
    let path2 = paths.next().map(str::to_string);

    let (loc_added, loc_deleted) = parse_numstat_line(numstat).unwrap_or((0, 0));

    if status.starts_with('R') {
        let similarity = parse_similarity(status);
        let from = path1;
        let path = path2?;
        return Some(FileChange {
            path,
            change_type: ChangeType::Renamed { from, similarity },
            loc_added,
            loc_deleted,
            hunks: vec![],
        });
    }
    if status.starts_with('C') {
        let similarity = parse_similarity(status);
        let from = path1;
        let path = path2?;
        return Some(FileChange {
            path,
            change_type: ChangeType::Copied { from, similarity },
            loc_added,
            loc_deleted,
            hunks: vec![],
        });
    }

    let change_type = match status {
        "A" => ChangeType::Added,
        "D" => ChangeType::Deleted,
        "M" => ChangeType::Modified,
        _ => ChangeType::BinaryOrUnknown,
    };

    Some(FileChange {
        path: path1,
        change_type,
        loc_added,
        loc_deleted,
        hunks: vec![],
    })
}

/// Parse a numstat line. Returns `None` for malformed input. Binary files
/// emit `-\t-\tpath`; we coerce both to zero (lossy by design — we don't
/// have line counts for binaries, and zero is the honest value).
fn parse_numstat_line(line: &str) -> Option<(u32, u32)> {
    let mut cols = line.split('\t');
    let added_str = cols.next()?;
    let deleted_str = cols.next()?;
    let added = if added_str == "-" {
        0
    } else {
        added_str.parse().ok()?
    };
    let deleted = if deleted_str == "-" {
        0
    } else {
        deleted_str.parse().ok()?
    };
    Some((added, deleted))
}

fn parse_similarity(status: &str) -> u8 {
    status[1..].parse::<u8>().unwrap_or(100)
}

/// Parse `@@ -old_start[,old_lines] +new_start[,new_lines] @@` hunk headers
/// from unified diff output.
fn parse_hunk_headers(raw: &str) -> Vec<Hunk> {
    raw.lines()
        .filter(|l| l.starts_with("@@"))
        .filter_map(parse_hunk_header_line)
        .collect()
}

fn parse_hunk_header_line(line: &str) -> Option<Hunk> {
    // Format: "@@ -a[,b] +c[,d] @@[ optional context]"
    // After the leading "@@" there is a space, then "-a,b +c,d", then " @@".
    let after_at = line.strip_prefix("@@")?.trim_start();
    let end = after_at.find(" @@")?;
    let range_str = &after_at[..end];

    let mut parts = range_str.split_whitespace();
    let old_part = parts.next()?; // e.g. "-1,3"
    let new_part = parts.next()?; // e.g. "+5,2"

    let (old_start, old_lines) = parse_range(old_part.strip_prefix('-')?)?;
    let (new_start, new_lines) = parse_range(new_part.strip_prefix('+')?)?;

    Some(Hunk {
        old_start,
        old_lines,
        new_start,
        new_lines,
    })
}

fn parse_range(s: &str) -> Option<(u32, u32)> {
    if let Some((start, count)) = s.split_once(',') {
        Some((start.parse().ok()?, count.parse().ok()?))
    } else {
        // No comma: count defaults to 1
        Some((s.parse().ok()?, 1))
    }
}

fn parse_iso_date(s: &str) -> Option<Date> {
    // ISO 8601 with offset, e.g. "2026-06-06T19:29:04+02:00"
    // The `time` crate is compiled without the `parsing` feature, so we parse
    // the date portion (YYYY-MM-DD) manually from the leading 10 characters.
    use time::Month;
    let s = s.trim();
    if s.len() < 10 {
        return None;
    }
    let year: i32 = s[0..4].parse().ok()?;
    let month: u8 = s[5..7].parse().ok()?;
    let day: u8 = s[8..10].parse().ok()?;
    let month = Month::try_from(month).ok()?;
    Date::from_calendar_date(year, month, day).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reproducer for the bug where `parse_git_log_stream` silently drops the commit
    /// immediately following a merge commit that has an empty change block.
    ///
    /// The hand-crafted stream mirrors what `git log --raw --numstat
    /// --pretty=format:%H%x1f%P%x1f%ae%x1f%an%x1f%ce%x1f%aI%x1f%B%x1e`
    /// emits for three commits: a regular commit, a no-ff merge with no
    /// file changes, and a subsequent regular commit.
    ///
    /// Splitting on `\x1e` yields four chunks:
    ///   `chunk[0]` = `A_pretty`
    ///   `chunk[1]` = `\nA_changes\n\nB_pretty`
    ///   `chunk[2]` = `\nC_pretty` ← merge has no changes, so no `\n\n`
    ///   `chunk[3]` = `\nC_changes\n\n`
    ///
    /// Before the fix, `split_off_name_status_prefix(chunk[2])` found no `\n\n` and
    /// returned `""`, causing commit C to be silently dropped.
    #[test]
    fn parser_does_not_drop_commit_after_empty_name_status_merge() {
        // Fake but structurally valid SHAs (40 hex chars each).
        let sha_a = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let sha_b = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let sha_x = "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"; // branch tip merged into B
        let sha_c = "cccccccccccccccccccccccccccccccccccccccc";

        // Raw + numstat per commit. The `:`-prefixed line is the raw
        // diff line (status + paths); the bare line after is the numstat
        // line (added \t deleted \t path).
        let raw = format!(
            "{sha_a}\x1f\x1fa@x\x1fAlice\x1fce@x\x1f2025-01-01T00:00:00+00:00\x1fmsg A\n\x1e\n:100644 100644 a000 a001 M\tsrc/foo.rs\n3\t1\tsrc/foo.rs\n\n\
            {sha_b}\x1f{sha_a} {sha_x}\x1fa@x\x1fAlice\x1fce@x\x1f2025-01-02T00:00:00+00:00\x1fmsg B\n\x1e\n\
            {sha_c}\x1f{sha_b}\x1fa@x\x1fAlice\x1fce@x\x1f2025-01-03T00:00:00+00:00\x1fmsg C\n\x1e\n:000000 100644 0000 c001 A\tsrc/bar.rs\n5\t0\tsrc/bar.rs\n\n"
        );

        let events = parse_git_log_stream(&raw);

        let revs: Vec<&str> = events.iter().map(|e| e.rev.as_str()).collect();
        assert_eq!(
            revs,
            vec![sha_a, sha_b, sha_c],
            "expected all 3 commits; got: {revs:?}"
        );

        // Commit A should have one changed file.
        assert_eq!(
            events[0].changes.len(),
            1,
            "commit A should have 1 changed file"
        );

        // Commit B (merge) should have zero changed files.
        assert_eq!(
            events[1].changes.len(),
            0,
            "merge commit B should have 0 changed files"
        );
        assert_eq!(
            events[1].parents.len(),
            2,
            "merge commit B should have 2 parents"
        );

        // Commit C should have one changed file.
        assert_eq!(
            events[2].changes.len(),
            1,
            "commit C should have 1 changed file"
        );

        // Plumbed numstat values must reach the FileChange — the whole
        // point of the --raw --numstat switch. Commit A's foo.rs gained 3
        // lines and lost 1.
        assert_eq!(
            events[0].changes[0].loc_added, 3,
            "commit A foo.rs loc_added"
        );
        assert_eq!(
            events[0].changes[0].loc_deleted, 1,
            "commit A foo.rs loc_deleted"
        );
        // Commit C added bar.rs with 5 lines (deleted=0).
        assert_eq!(
            events[2].changes[0].loc_added, 5,
            "commit C bar.rs loc_added"
        );
        assert_eq!(
            events[2].changes[0].loc_deleted, 0,
            "commit C bar.rs loc_deleted"
        );
    }
}
