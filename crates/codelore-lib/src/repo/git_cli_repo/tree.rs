//! `git ls-tree -r -z` record parsing for `GitCliRepo::tracked_paths_at_head`.

/// Parse one NUL-terminated `git ls-tree -r -z` record
/// (`<mode> <type> <oid>\t<path>`) into its path when the mode is a
/// regular-file blob — the `0o100xxx` mode class, matched by prefix so
/// legacy non-canonical modes (e.g. group-writable `100664` trees
/// predating git's normalization) are kept, exactly as `GixRepo`'s
/// class-based `is_blob` filter keeps them. Symlinks (`120000`) and
/// submodule gitlinks (`160000`) return `None`. The path bytes are
/// decoded lossily, mirroring how `GixRepo` renders its `BString`
/// paths — both backends therefore produce identical strings for any
/// valid-UTF-8 path.
pub(super) fn parse_ls_tree_record(record: &[u8]) -> Option<String> {
    let tab = record.iter().position(|b| *b == b'\t')?;
    let mode = record[..tab].split(|b| *b == b' ').next()?;
    if !mode.starts_with(b"100") {
        return None;
    }
    Some(String::from_utf8_lossy(&record[tab + 1..]).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `parse_ls_tree_record` keeps regular-file blobs (100644/100755)
    /// and preserves paths with spaces (the `-z` NUL framing means the
    /// tab is the only in-record separator before the path).
    #[test]
    fn ls_tree_record_keeps_regular_file_blobs() {
        assert_eq!(
            parse_ls_tree_record(
                b"100644 blob d00491fd7e5bb6fa28c517a0bb32b8b506539d4d\tsrc/main file.rs"
            ),
            Some("src/main file.rs".to_string()),
        );
        assert_eq!(
            parse_ls_tree_record(
                b"100755 blob d00491fd7e5bb6fa28c517a0bb32b8b506539d4d\tscripts/run.sh"
            ),
            Some("scripts/run.sh".to_string()),
        );
        // Legacy non-canonical blob mode (pre-normalization trees) stays a
        // regular file — the class prefix matches, mirroring gix `is_blob`.
        assert_eq!(
            parse_ls_tree_record(
                b"100664 blob d00491fd7e5bb6fa28c517a0bb32b8b506539d4d\tlegacy.rs"
            ),
            Some("legacy.rs".to_string()),
        );
    }

    /// Symlinks (120000) and submodule gitlinks (160000) must be dropped
    /// — mirrors `GixRepo`'s `is_blob` mode filter. Empty trailing
    /// records (from the final NUL) parse to `None` too.
    #[test]
    fn ls_tree_record_drops_symlinks_gitlinks_and_empty() {
        assert_eq!(
            parse_ls_tree_record(b"120000 blob d00491fd7e5bb6fa28c517a0bb32b8b506539d4d\tlink.rs"),
            None,
        );
        assert_eq!(
            parse_ls_tree_record(
                b"160000 commit d00491fd7e5bb6fa28c517a0bb32b8b506539d4d\tvendor/dep"
            ),
            None,
        );
        assert_eq!(parse_ls_tree_record(b""), None);
    }
}
