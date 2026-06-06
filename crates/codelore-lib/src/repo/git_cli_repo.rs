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
            "--name-status",
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

        let events: Vec<Result<CommitEvent>> =
            parse_git_log_stream(&raw).into_iter().map(Ok).collect();

        Ok(Box::new(events.into_iter()))
    }

    fn changed_files(&self, rev: &str) -> Result<Vec<FileChange>> {
        let output = self.run_git(&["show", "--name-status", "--pretty=format:", rev])?;
        if !output.status.success() {
            return Err(CodeLoreError::Repo(format!(
                "git show: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }
        let raw = String::from_utf8(output.stdout)
            .map_err(|e| CodeLoreError::Repo(format!("git show output not utf-8: {e}")))?;
        Ok(parse_name_status(&raw))
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

/// From a chunk that starts with `\n<name-status>\n\n<pretty>`, return the pretty portion.
fn split_off_name_status_prefix(chunk: &str) -> &str {
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
fn extract_name_status_prefix(chunk: &str) -> &str {
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
    let changes = parse_name_status(name_status);

    Some(CommitEvent {
        rev: sha,
        author_email,
        author_name,
        committer_email,
        date,
        message,
        parents,
        changes,
        canonical_author: None, // Not resolved at walk time; matches GixRepo behaviour.
        ai_attribution: None,   // Not classified at walk time; matches GixRepo behaviour.
        kamei: None,
    })
}

/// Parse a name-status block such as:
/// ```text
/// M\tsrc/main.rs
/// A\tsrc/lib.rs
/// R90\tsrc/old.rs\tsrc/new.rs
/// D\tsrc/gone.rs
/// ```
fn parse_name_status(raw: &str) -> Vec<FileChange> {
    raw.lines().filter_map(parse_name_status_line).collect()
}

fn parse_name_status_line(line: &str) -> Option<FileChange> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let mut cols = line.splitn(3, '\t');
    let status = cols.next()?;
    let path1 = cols.next()?.to_string();
    let path2 = cols.next().map(str::to_string);

    let change_type = if status.starts_with('R') {
        let similarity = parse_similarity(status);
        let from = path1.clone();
        let path = path2?;
        return Some(FileChange {
            path,
            change_type: ChangeType::Renamed { from, similarity },
            loc_added: 0,
            loc_deleted: 0,
            hunks: vec![],
        });
    } else if status.starts_with('C') {
        let similarity = parse_similarity(status);
        let from = path1.clone();
        let path = path2?;
        return Some(FileChange {
            path,
            change_type: ChangeType::Copied { from, similarity },
            loc_added: 0,
            loc_deleted: 0,
            hunks: vec![],
        });
    } else {
        match status {
            "A" => ChangeType::Added,
            "D" => ChangeType::Deleted,
            "M" => ChangeType::Modified,
            _ => ChangeType::BinaryOrUnknown,
        }
    };

    Some(FileChange {
        path: path1,
        change_type,
        loc_added: 0,
        loc_deleted: 0,
        hunks: vec![],
    })
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
