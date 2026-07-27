//! `git status --porcelain=v2 -z` parsing for `GitCliRepo::worktree_changes`.

use crate::repo::WORKTREE_CONFLICT_MESSAGE;
use crate::{CodeLoreError, Result};

/// A parsed `git status --porcelain=v2 -z` snapshot: one mode-filtered
/// entry per candidate path, with rename destinations carrying their
/// source and rename sources expanded into entries of their own.
pub(super) type ParsedStatus = Vec<StatusEntry>;

/// One candidate path from a porcelain-v2 status record, before net
/// classification.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct StatusEntry {
    pub(super) path: String,
    /// The rename source when this entry is the destination of a `2 R…`
    /// record; `None` otherwise (including copy destinations — a copy's
    /// source still exists unchanged).
    pub(super) rename_from: Option<String>,
}

/// Parse `git status --porcelain=v2 -z --untracked-files=no` output.
///
/// Record forms (each NUL-terminated; `-z` uses NUL both between and
/// within records):
///   - `1 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <path>` — ordinary change.
///   - `2 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <X><score> <path>` followed
///     by a SECOND NUL-separated field holding the ORIGINAL path — the new
///     path comes first, then the source.
///   - `u <XY> <sub> <m1> <m2> <m3> <mW> <h1> <h2> <h3> <path>` — unmerged.
///
/// Symlink (`120000`) and submodule-gitlink (`160000`) modes are filtered
/// per contribution, mirroring `GixRepo`'s index-entry-mode filter: a
/// staged deletion is judged by its HEAD mode `mH`, every other
/// contribution by the index mode `mI`. Unmerged records abort with an
/// error — a conflicted tree cannot be net-classified against HEAD.
/// Unrecognized records are skipped, matching the other parsers in this
/// file.
pub(super) fn parse_porcelain_v2(bytes: &[u8]) -> Result<ParsedStatus> {
    let mut entries = Vec::new();
    let mut tokens = bytes.split(|b| *b == 0);
    while let Some(token) = tokens.next() {
        if token.is_empty() {
            continue;
        }
        let record = String::from_utf8_lossy(token);
        if record.starts_with("u ") {
            return Err(CodeLoreError::Analysis(WORKTREE_CONFLICT_MESSAGE.into()));
        }
        if let Some(rest) = record.strip_prefix("1 ") {
            entries.extend(parse_v2_ordinary(rest));
        } else if let Some(rest) = record.strip_prefix("2 ") {
            // Rename/copy records span TWO NUL-separated fields; consume
            // the second one here so it is never misread as a record.
            let original = tokens.next().ok_or_else(|| {
                CodeLoreError::Repo(
                    "git status --porcelain=v2: rename record missing its source-path field".into(),
                )
            })?;
            let original = String::from_utf8_lossy(original);
            if let Some(parsed) = parse_v2_rename(rest, &original) {
                entries.extend(parsed);
            }
        }
    }
    Ok(entries)
}

/// True for the porcelain-v2 octal modes that `worktree_changes` excludes:
/// symlinks and submodule gitlinks, matched via the file-type class bits —
/// the same rule `GixRepo` applies to index-entry modes.
fn is_symlink_or_gitlink_mode(mode: u32) -> bool {
    matches!(mode & 0o170_000, 0o120_000 | 0o160_000)
}

/// Parse the remainder of a `1 ` record (prefix already stripped) into a
/// candidate entry, or `None` when malformed or when every contributing
/// side is a symlink/gitlink mode.
fn parse_v2_ordinary(rest: &str) -> Option<StatusEntry> {
    let mut fields = rest.splitn(8, ' ');
    let mut xy = fields.next()?.chars();
    let staged = xy.next()?;
    let unstaged = xy.next()?;
    let _sub = fields.next()?;
    let mode_head = u32::from_str_radix(fields.next()?, 8).ok()?;
    let mode_index = u32::from_str_radix(fields.next()?, 8).ok()?;
    let _mode_worktree = fields.next()?;
    let _hash_head = fields.next()?;
    let _hash_index = fields.next()?;
    let path = fields.next()?;
    // A staged deletion has no index entry left, so its mode filter judges
    // the HEAD side; everything else is judged by the index mode — the
    // exact per-stream rule `GixRepo` gets from `entry_mode`.
    let staged_mode = if staged == 'D' { mode_head } else { mode_index };
    let staged_contributes = staged != '.' && !is_symlink_or_gitlink_mode(staged_mode);
    let unstaged_contributes = unstaged != '.' && !is_symlink_or_gitlink_mode(mode_index);
    (staged_contributes || unstaged_contributes).then(|| StatusEntry {
        path: path.to_string(),
        rename_from: None,
    })
}

/// Parse the remainder of a `2 ` record (prefix already stripped) plus its
/// second NUL field (the original path). A rename yields the destination
/// (carrying `rename_from`) AND the source as its own entry; a copy yields
/// only the destination. Returns `None` when malformed, and an empty list
/// when the entry is filtered by mode.
fn parse_v2_rename(rest: &str, original_path: &str) -> Option<Vec<StatusEntry>> {
    let mut fields = rest.splitn(9, ' ');
    let _xy = fields.next()?;
    let _sub = fields.next()?;
    let _mode_head = fields.next()?;
    let mode_index = u32::from_str_radix(fields.next()?, 8).ok()?;
    let _mode_worktree = fields.next()?;
    let _hash_head = fields.next()?;
    let _hash_index = fields.next()?;
    let score = fields.next()?;
    let path = fields.next()?;
    if is_symlink_or_gitlink_mode(mode_index) {
        return Some(Vec::new());
    }
    Some(if score.starts_with('R') {
        vec![
            StatusEntry {
                path: path.to_string(),
                rename_from: Some(original_path.to_string()),
            },
            StatusEntry {
                path: original_path.to_string(),
                rename_from: None,
            },
        ]
    } else {
        // A copy's source still exists unchanged; only the destination is
        // a candidate.
        vec![StatusEntry {
            path: path.to_string(),
            rename_from: None,
        }]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ordinary `1 ` records: unstaged, staged, and both-stages entries
    /// each yield one candidate; the record layout is
    /// `1 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <path>` NUL-terminated.
    #[test]
    fn porcelain_v2_ordinary_records_yield_one_entry_each() {
        let raw = "1 .M N... 100644 100644 100644 aaaa bbbb src/unstaged.rs\0\
                   1 M. N... 100644 100644 100644 aaaa bbbb src/staged.rs\0\
                   1 MM N... 100644 100644 100644 aaaa bbbb src/both.rs\0";
        let entries = parse_porcelain_v2(raw.as_bytes()).expect("parse");
        let paths: Vec<&str> = entries.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, ["src/unstaged.rs", "src/staged.rs", "src/both.rs"]);
        assert!(entries.iter().all(|e| e.rename_from.is_none()));
    }

    /// A staged deletion has `mI = 000000`, so its mode filter must judge
    /// the HEAD mode: a deleted regular file stays a candidate while a
    /// deleted symlink is dropped.
    #[test]
    fn porcelain_v2_staged_delete_judged_by_head_mode() {
        let raw = "1 D. N... 100644 000000 000000 aaaa 0000 gone.rs\0\
                   1 D. N... 120000 000000 000000 aaaa 0000 gone-link\0";
        let entries = parse_porcelain_v2(raw.as_bytes()).expect("parse");
        assert_eq!(
            entries,
            vec![StatusEntry {
                path: "gone.rs".to_string(),
                rename_from: None,
            }],
        );
    }

    /// Symlink (`120000`) and submodule-gitlink (`160000`) index modes are
    /// filtered out — mirroring `GixRepo`'s index-entry-mode filter.
    #[test]
    fn porcelain_v2_filters_symlinks_and_gitlinks() {
        let raw = "1 .M N... 120000 120000 120000 aaaa bbbb link\0\
                   1 .M S.M. 160000 160000 160000 aaaa bbbb vendor/dep\0";
        let entries = parse_porcelain_v2(raw.as_bytes()).expect("parse");
        assert_eq!(entries, Vec::new());
    }

    /// An intent-to-add entry (`git add -N`) renders `mI = 000000`, which
    /// must NOT be mistaken for a filtered mode — the worktree file is a
    /// real candidate.
    #[test]
    fn porcelain_v2_intent_to_add_is_kept() {
        let raw = "1 .A N... 000000 000000 100644 0000 0000 ita.rs\0";
        let entries = parse_porcelain_v2(raw.as_bytes()).expect("parse");
        assert_eq!(
            entries,
            vec![StatusEntry {
                path: "ita.rs".to_string(),
                rename_from: None,
            }],
        );
    }

    /// A `2 R…` rename record spans TWO NUL fields — NEW path first, then
    /// the original. It must yield the destination (carrying
    /// `rename_from`) plus the source as its own entry, and the record
    /// AFTER it must still parse (the two-field consumption cannot slip).
    #[test]
    fn porcelain_v2_rename_record_reads_two_fields() {
        let raw = "2 R. N... 100644 100644 100644 aaaa bbbb R100 new.rs\0old.rs\0\
                   1 .M N... 100644 100644 100644 aaaa bbbb after.rs\0";
        let entries = parse_porcelain_v2(raw.as_bytes()).expect("parse");
        assert_eq!(
            entries,
            vec![
                StatusEntry {
                    path: "new.rs".to_string(),
                    rename_from: Some("old.rs".to_string()),
                },
                StatusEntry {
                    path: "old.rs".to_string(),
                    rename_from: None,
                },
                StatusEntry {
                    path: "after.rs".to_string(),
                    rename_from: None,
                },
            ],
        );
    }

    /// An unmerged (`u `) record must abort the parse with an `Analysis`
    /// error — a conflicted tree cannot be net-classified against HEAD.
    #[test]
    fn porcelain_v2_unmerged_record_errors() {
        let raw = "u UU N... 100644 100644 100644 100644 aaaa bbbb cccc conflicted.rs\0";
        let err = parse_porcelain_v2(raw.as_bytes()).expect_err("must error");
        match err {
            CodeLoreError::Analysis(msg) => {
                assert!(
                    msg.contains("unmerged paths"),
                    "error must name the unmerged state: {msg}"
                );
            }
            other => panic!("expected CodeLoreError::Analysis, got {other:?}"),
        }
    }

    /// Empty status output (a clean tree) parses to no entries.
    #[test]
    fn porcelain_v2_empty_output_is_no_entries() {
        assert_eq!(parse_porcelain_v2(b"").expect("parse"), Vec::new());
    }
}
