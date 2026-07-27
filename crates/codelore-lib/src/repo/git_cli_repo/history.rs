//! Parsers for `git log` / `git show` textual output: the commit-event
//! stream, per-commit raw+numstat change block, rename-path expansion, and
//! unified-diff hunk headers behind `GitCliRepo`'s history-walking methods.

use crate::{ChangeType, CommitEvent, FileChange, Hunk};

use super::parse_iso_timestamp;

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
pub(super) fn parse_git_log_stream(raw: &str) -> Vec<CommitEvent> {
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
    // pretty is: SHA\x1fPARENTS\x1fAE\x1fAN\x1fCE\x1fAISO\x1fCISO\x1fMESSAGE\n
    let mut parts = pretty.splitn(8, '\x1f');
    let sha = parts.next()?.trim().to_string();
    if sha.is_empty() || sha.len() < 7 {
        return None;
    }
    let parents_raw = parts.next()?.trim().to_string();
    let author_email = parts.next()?.trim().to_string();
    let author_name = parts.next()?.trim().to_string();
    let committer_email = parts.next()?.trim().to_string();
    let date_str = parts.next()?.trim().to_string();
    let committer_date_str = parts.next()?.trim().to_string();
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

    let date = parse_iso_timestamp(&date_str)?;
    let committer_date = parse_iso_timestamp(&committer_date_str)?;
    let changes = parse_changes_block(name_status);

    Some(CommitEvent {
        rev: sha,
        author_email,
        author_name,
        committer_email,
        date,
        committer_date,
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
/// Previously we zipped raw and numstat lines positionally.
/// That assumption broke on commits with submodule additions/removals
/// or binary exclusions where the two streams diverge in length — the
/// trailing entries got dropped silently and any pre-divergence
/// mismatch corrupted ALL subsequent rows' line counts. Now we
/// HashMap-join by the destination path key (extracted from both
/// streams), tolerating any per-stream length difference cleanly.
pub(super) fn parse_changes_block(block: &str) -> Vec<FileChange> {
    use std::collections::HashMap;

    let mut raw_entries: Vec<&str> = Vec::new();
    let mut numstat_by_path: HashMap<String, (u32, u32)> = HashMap::new();
    for line in block.lines() {
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(raw) = trimmed.strip_prefix(':') {
            raw_entries.push(raw);
        } else if let Some((key, stat)) = parse_numstat_with_key(trimmed) {
            numstat_by_path.insert(key, stat);
        }
        // Unparseable lines are silently skipped — same as the previous
        // behaviour, just no longer corrupts subsequent rows.
    }
    raw_entries
        .into_iter()
        .filter_map(|raw| {
            let key = raw_destination_path(raw)?;
            let stat = numstat_by_path.remove(&key).unwrap_or((0, 0));
            parse_raw_with_stat(raw, stat)
        })
        .collect()
}

/// Parse a numstat line into `(destination_path, (added, deleted))`.
/// Returns `None` on malformed input.
///
/// The destination path is the key for joining against the raw line
/// stream. Git's numstat emits renames in two shapes:
///
///   1. Whole-path arrow: `old/path => new/path` (paths share no common
///      affix).
///   2. Brace-collapsed: `prefix/{old => new}/suffix` (git collapses
///      the unchanged common prefix and suffix into braces). The
///      destination path is then `prefix/new/suffix`. Either side of
///      the arrow inside the braces can be empty — `a/{ => sub}/b.rs`
///      means the destination is `a/sub/b.rs` and the source was
///      `a/b.rs`.
fn parse_numstat_with_key(line: &str) -> Option<(String, (u32, u32))> {
    let mut cols = line.split('\t');
    let added_str = cols.next()?;
    let deleted_str = cols.next()?;
    let path_part = cols.next()?;
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
    let key = expand_rename_path_destination(path_part);
    Some((key, (added, deleted)))
}

/// Resolve git's rename path syntax to the destination path. See
/// [`parse_numstat_with_key`] for the shapes git emits. Handles the
/// brace-collapsed form by stripping `{old => new}` segments down to
/// `new` and gracefully collapsing the resulting `//` if `new` is
/// empty. Non-rename inputs pass through unchanged.
fn expand_rename_path_destination(path: &str) -> String {
    // Brace form first: outer-most `{ … => … }` wins. There's at most
    // one brace pair per path in git's output.
    if let (Some(open), Some(close)) = (path.find('{'), path.rfind('}'))
        && open < close
        && let Some((_, after_arrow)) = path[open + 1..close].split_once(" => ")
    {
        let prefix = &path[..open];
        let suffix = &path[close + 1..];
        let mut out = String::with_capacity(prefix.len() + after_arrow.len() + suffix.len());
        out.push_str(prefix);
        out.push_str(after_arrow);
        out.push_str(suffix);
        // Collapse the `//` that appears when `after_arrow` is empty
        // and prefix already ends with `/` (e.g. `a/{ => sub}/b.rs`
        // with new side empty becomes `a//b.rs`; want `a/b.rs`).
        return out.replace("//", "/");
    }
    // Whole-path arrow form: `old => new`.
    if let Some((_, new_path)) = path.split_once(" => ") {
        return new_path.to_string();
    }
    path.to_string()
}

/// Extract the destination path from a raw line (prefix `:` already
/// stripped). For `R`/`C` statuses (rename/copy) the destination is
/// `path2`; for everything else (`A`/`D`/`M`/`T`/`B`/`U`) it's `path1`.
/// Used as the join key against numstat entries.
fn raw_destination_path(raw: &str) -> Option<String> {
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
    if status.starts_with('R') || status.starts_with('C') {
        path2
    } else {
        Some(path1)
    }
}

/// Parse a single raw line paired with its pre-extracted numstat tuple.
///
/// Raw format: `<mode1> <mode2> <hash1> <hash2> <STATUS>\t<path>[\t<path2>]`
/// (the leading `:` has already been stripped by the caller). The
/// `(loc_added, loc_deleted)` tuple comes from `parse_numstat_with_key`
/// joined by destination path — see `parse_changes_block` for the
/// rationale.
fn parse_raw_with_stat(raw: &str, stat: (u32, u32)) -> Option<FileChange> {
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

    let (loc_added, loc_deleted) = stat;

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

fn parse_similarity(status: &str) -> u8 {
    status[1..].parse::<u8>().unwrap_or(100)
}

/// Parse `@@ -old_start[,old_lines] +new_start[,new_lines] @@` hunk headers
/// from unified diff output.
pub(super) fn parse_hunk_headers(raw: &str) -> Vec<Hunk> {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Brace-collapsed renames at various positions in the path must
    /// expand to the destination so the numstat key joins correctly
    /// against the raw stream (which always names the destination
    /// path in its `R` line).
    #[test]
    fn brace_rename_at_path_root() {
        assert_eq!(
            expand_rename_path_destination("{old.rs => new.rs}"),
            "new.rs"
        );
    }

    #[test]
    fn brace_rename_with_common_prefix() {
        assert_eq!(
            expand_rename_path_destination("src/{old.rs => new.rs}"),
            "src/new.rs"
        );
    }

    #[test]
    fn brace_rename_with_common_prefix_and_suffix() {
        assert_eq!(
            expand_rename_path_destination("src/{old => new}/file.rs"),
            "src/new/file.rs"
        );
    }

    #[test]
    fn brace_rename_empty_new_side_collapses_slash() {
        // `a/{sub => }/b.rs` means destination is `a/b.rs`. Without the
        // // -> / collapse the result would be `a//b.rs`.
        assert_eq!(expand_rename_path_destination("a/{sub => }/b.rs"), "a/b.rs");
    }

    #[test]
    fn brace_rename_empty_old_side() {
        assert_eq!(
            expand_rename_path_destination("a/{ => sub}/b.rs"),
            "a/sub/b.rs"
        );
    }

    #[test]
    fn whole_path_arrow_rename_passes_through() {
        assert_eq!(
            expand_rename_path_destination("src/old.rs => other/new.rs"),
            "other/new.rs"
        );
    }

    #[test]
    fn non_rename_path_unchanged() {
        assert_eq!(expand_rename_path_destination("src/main.rs"), "src/main.rs");
    }

    /// `parse_numstat_with_key` end-to-end on a brace rename.
    #[test]
    fn numstat_with_brace_rename_produces_destination_key() {
        let (key, stat) = parse_numstat_with_key("12\t3\tsrc/{old.rs => new.rs}").expect("parse");
        assert_eq!(key, "src/new.rs");
        assert_eq!(stat, (12, 3));
    }

    // Path-key extraction must match between raw and
    // numstat streams so HashMap-join can correctly pair them.

    #[test]
    fn f8_numstat_key_plain_path() {
        let (key, stat) = parse_numstat_with_key("12\t3\tsrc/main.rs").expect("parse");
        assert_eq!(key, "src/main.rs");
        assert_eq!(stat, (12, 3));
    }

    #[test]
    fn f8_numstat_key_rename_uses_destination() {
        // numstat rename form: `<added>\t<deleted>\t<old> => <new>`
        let (key, _stat) = parse_numstat_with_key("0\t0\tsrc/old.rs => src/new.rs").expect("parse");
        assert_eq!(key, "src/new.rs", "rename key must be the destination");
    }

    #[test]
    fn f8_numstat_key_binary_files_normalized_to_zero() {
        let (_key, stat) = parse_numstat_with_key("-\t-\tassets/logo.png").expect("parse");
        assert_eq!(stat, (0, 0), "binary `-\\t-` markers must coerce to 0 LoC");
    }

    #[test]
    fn f8_raw_destination_uses_path2_for_rename() {
        let raw = "100644 100644 d00491f 2b2f2e1 R100\tsrc/old.rs\tsrc/new.rs";
        assert_eq!(
            raw_destination_path(raw).expect("parse"),
            "src/new.rs",
            "rename destination = path2",
        );
    }

    #[test]
    fn f8_raw_destination_uses_path1_for_modify() {
        let raw = "100644 100644 d00491f 2b2f2e1 M\tsrc/main.rs";
        assert_eq!(
            raw_destination_path(raw).expect("parse"),
            "src/main.rs",
            "modify uses path1 (only path present)",
        );
    }

    /// When raw and numstat streams have UNEQUAL lengths
    /// (the original positional zip dropped trailing entries and
    /// corrupted preceding ones), HashMap-join must still produce
    /// correct line counts for every raw entry. We simulate a submodule
    /// addition: raw stream has 2 entries, numstat only has 1.
    #[test]
    fn f8_unequal_raw_and_numstat_no_corruption() {
        let block = ":100644 100644 d00491f 2b2f2e1 M\tsrc/main.rs\n\
                     :160000 160000 0000000 1111111 M\tvendor/libfoo\n\
                     12\t3\tsrc/main.rs\n";
        let changes = parse_changes_block(block);
        assert_eq!(
            changes.len(),
            2,
            "both raw entries must surface (1 with stats, 1 with default 0/0)"
        );
        let main_rs = changes
            .iter()
            .find(|c| c.path == "src/main.rs")
            .expect("src/main.rs surfaces");
        assert_eq!(
            (main_rs.loc_added, main_rs.loc_deleted),
            (12, 3),
            "src/main.rs must NOT be corrupted by the submodule's missing numstat line",
        );
        let submodule = changes
            .iter()
            .find(|c| c.path == "vendor/libfoo")
            .expect("submodule surfaces");
        assert_eq!(
            (submodule.loc_added, submodule.loc_deleted),
            (0, 0),
            "submodule missing numstat → default 0/0 (not corrupted by zip-shift)",
        );
    }

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
            "{sha_a}\x1f\x1fa@x\x1fAlice\x1fce@x\x1f2025-01-01T00:00:00+00:00\x1f2025-01-01T00:00:00+00:00\x1fmsg A\n\x1e\n:100644 100644 a000 a001 M\tsrc/foo.rs\n3\t1\tsrc/foo.rs\n\n\
            {sha_b}\x1f{sha_a} {sha_x}\x1fa@x\x1fAlice\x1fce@x\x1f2025-01-02T00:00:00+00:00\x1f2025-01-02T00:00:00+00:00\x1fmsg B\n\x1e\n\
            {sha_c}\x1f{sha_b}\x1fa@x\x1fAlice\x1fce@x\x1f2025-01-03T00:00:00+00:00\x1f2025-01-03T00:00:00+00:00\x1fmsg C\n\x1e\n:000000 100644 0000 c001 A\tsrc/bar.rs\n5\t0\tsrc/bar.rs\n\n"
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
