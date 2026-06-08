//! Path-string utilities. Centralised here so the two HEAD-time walks
//! (`facts::ingest::populate_clones_at_head` and `analyses::clones`'s
//! relative-path helper) use the same forward-slash normalisation rule —
//! before this lived in two places, and a Windows-specific bug in
//! `clone_coupling`'s `same_parent_dir` filter (rfind('/')) could
//! resurface if one site forgot to normalise.

use std::path::Path;

/// Convert a path to its POSIX (`/`-separator) string form. Git emits
/// `/` in its log output unconditionally, so every path stored in
/// `changes.path` / `clones.path` must also use `/` regardless of host
/// platform. On Unix this is a no-op (`MAIN_SEPARATOR == '/'`); on
/// Windows it rewrites `\` to `/`.
#[must_use]
pub fn to_posix(path: &Path) -> String {
    path.to_string_lossy()
        .replace(std::path::MAIN_SEPARATOR, "/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn unix_paths_pass_through_unchanged() {
        let p = PathBuf::from("src/foo/bar.rs");
        assert_eq!(to_posix(&p), "src/foo/bar.rs");
    }

    #[test]
    fn normalises_native_separator_to_forward_slash() {
        // Build a path via PathBuf::push so this exercises the actual
        // native separator on the host platform; on Unix it stays `/`,
        // on Windows it becomes `\` then gets rewritten back to `/`.
        let mut p = PathBuf::new();
        p.push("src");
        p.push("foo");
        p.push("bar.rs");
        assert_eq!(to_posix(&p), "src/foo/bar.rs");
    }
}
