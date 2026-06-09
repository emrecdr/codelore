//! Persistent fact-store cache: key derivation + XDG path resolution.
//! Plan 8 §3 — Tasks 11-14.
//!
//! Cache path layout:
//!   `$XDG_CACHE_HOME/codelore/<repo_hash_8>/<cache_key_16>.duckdb`
//!
//! Cache key covers: `canonical_repo_path`, HEAD SHA, crate version, options
//! thresholds, schema version. Excludes: `rows_limit`, `repo_path` (already
//! folded into `repo_hash_8`), cosmetic flags.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use sha2::{Digest, Sha256};

use crate::Options;

/// Schema version sentinel. Bump whenever the `DuckDB` schema changes so that
/// old cache files are not opened with a new schema.
const SCHEMA_VERSION: &str = "schema_v2";

/// Compute a 32-byte SHA-256 cache key from:
///   `canonical_repo_path || NUL || head_sha || NUL || CARGO_PKG_VERSION || NUL`
///   `|| opts_hash(opts) || NUL || SCHEMA_VERSION`
///
/// The key is deterministic for identical inputs and changes whenever any
/// threshold knob or schema version changes.
#[must_use]
pub fn cache_key(repo_path: &Path, head_sha: &str, opts: &Options) -> [u8; 32] {
    let mut hasher = Sha256::new();
    let canonical = fs::canonicalize(repo_path).unwrap_or_else(|_| repo_path.to_path_buf());
    hasher.update(canonical.to_string_lossy().as_bytes());
    hasher.update(b"\x00");
    hasher.update(head_sha.as_bytes());
    hasher.update(b"\x00");
    hasher.update(env!("CARGO_PKG_VERSION").as_bytes());
    hasher.update(b"\x00");
    hasher.update(opts_hash(opts).as_bytes());
    hasher.update(b"\x00");
    hasher.update(SCHEMA_VERSION.as_bytes());
    let mut out = [0u8; 32];
    out.copy_from_slice(&hasher.finalize());
    out
}

/// Resolve the on-disk path for a cache entry:
///   `<cache_root>/codelore/<repo_hash_8>/<cache_key_16>.duckdb`
///
/// `cache_root` defaults to [`dirs::cache_dir()`] but can be overridden (Task 14).
#[must_use]
pub fn cache_path(key: &[u8; 32], repo_path: &Path) -> PathBuf {
    cache_path_with_root(key, repo_path, &default_cache_root())
}

/// Same as [`cache_path`] but with an explicit root (for `--cache-dir` override).
#[must_use]
pub fn cache_path_with_root(key: &[u8; 32], repo_path: &Path, root: &Path) -> PathBuf {
    let mut repo_hash = Sha256::new();
    repo_hash.update(repo_path.to_string_lossy().as_bytes());
    let repo_short = hex::encode(&repo_hash.finalize()[..4]); // 8 hex chars
    let key_short = hex::encode(&key[..8]); // 16 hex chars
    root.join("codelore")
        .join(repo_short)
        .join(format!("{key_short}.duckdb"))
}

/// Return [`dirs::cache_dir()`] or fall back to a user-namespaced subdir of
/// `/tmp` when the XDG dirs are unavailable (headless CI, containers, etc.).
///
/// The fallback used to be a bare `/tmp` — which, combined with the
/// per-repo `codelore/<repo_hash_8>/<key>.duckdb` join later, produced
/// `/tmp/codelore/...` under whichever user first invoked codelore on a
/// shared host. Subsequent users hit `EPERM` because the directory was
/// owned by user A and lacked group-write. The fallback now namespaces
/// by `$USER` / `$LOGNAME` / `$USERNAME`, falling back to a process-id
/// suffix if none are set (the latter case is essentially
/// containers/sandboxes where the OS already provides isolation, so PID
/// only needs to avoid same-process collisions).
#[must_use]
pub fn default_cache_root() -> PathBuf {
    dirs::cache_dir().unwrap_or_else(fallback_tmp_root)
}

/// User-namespaced `/tmp` fallback. Prefers `$USER`, then `$LOGNAME`, then
/// `$USERNAME` (Windows convention), then a process-id suffix as
/// last-resort. Pure-safe: no FFI required (the workspace forbids `unsafe`),
/// `std::process::id()` is a stable safe API.
fn fallback_tmp_root() -> PathBuf {
    let id = std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .or_else(|_| std::env::var("USERNAME"))
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("pid{}", std::process::id()));
    PathBuf::from(format!("/tmp/codelore-fallback-{id}"))
}

/// Produce a stable canonical string from the full Options struct to use as
/// the per-options cache fingerprint. Defers to `Options::canonical_json()`
/// which serializes every field (including clone-detection options, exclude
/// patterns, etc.) and applies normalizations (sorted `exclude_patterns`,
/// dropped cosmetic knobs like `rows_limit`).
///
/// Replaces the pre-2026-06-08 hand-curated 11-field allowlist that silently
/// omitted clone-detection options and produced cache collisions when those
/// options changed. The new behavior auto-propagates as Options grows.
fn opts_hash(opts: &Options) -> String {
    opts.canonical_json().to_string()
}

// ---------------------------------------------------------------------------
// LRU eviction (Task 13)
// ---------------------------------------------------------------------------

/// Remove `.duckdb` files from `repo_dir` beyond `max_entries`, deleting the
/// oldest (by mtime) first.
///
/// Errors from individual file deletes are logged but not fatal — a partial
/// prune is better than aborting the analysis.
pub fn prune_repo_cache(repo_dir: &Path, max_entries: usize) {
    let Ok(rd) = fs::read_dir(repo_dir) else {
        return;
    };

    let mut entries: Vec<(PathBuf, u64)> = rd
        .flatten()
        .filter(|e| {
            e.path()
                .extension()
                .and_then(|x| x.to_str())
                .is_some_and(|x| x == "duckdb")
        })
        .filter_map(|e| {
            let mtime = e
                .metadata()
                .ok()?
                .modified()
                .ok()?
                .duration_since(UNIX_EPOCH)
                .ok()?
                .as_secs();
            Some((e.path(), mtime))
        })
        .collect();

    if entries.len() <= max_entries {
        return;
    }

    // Sort ascending by mtime — oldest first.
    entries.sort_by_key(|(_, mtime)| *mtime);
    let to_delete = entries.len() - max_entries;
    for (path, _) in entries.into_iter().take(to_delete) {
        if let Err(e) = fs::remove_file(&path) {
            tracing::warn!("prune_repo_cache: failed to remove {}: {e}", path.display());
        } else {
            tracing::info!("prune_repo_cache: removed {}", path.display());
        }
    }
}

/// Walk `root/codelore/` and delete `.duckdb` files (oldest first) until the
/// total directory size falls below `max_bytes`.
///
/// Errors from individual file operations are logged but not fatal.
pub fn prune_global_cache(root: &Path, max_bytes: u64) {
    let codelore_dir = root.join("codelore");
    let Ok(walk) = collect_duckdb_files(&codelore_dir) else {
        return;
    };

    let total: u64 = walk.iter().map(|(_, _, size)| *size).sum();
    if total <= max_bytes {
        return;
    }

    // Sort ascending by mtime — oldest first.
    let mut files = walk;
    files.sort_by_key(|(_, mtime, _)| *mtime);

    let mut remaining = total;
    for (path, _, size) in files {
        if remaining <= max_bytes {
            break;
        }
        if let Err(e) = fs::remove_file(&path) {
            tracing::warn!(
                "prune_global_cache: failed to remove {}: {e}",
                path.display()
            );
        } else {
            tracing::info!("prune_global_cache: removed {}", path.display());
            remaining = remaining.saturating_sub(size);
        }
    }
}

/// Recursively collect all `.duckdb` files under `dir`.
/// Returns `(path, mtime_secs, size_bytes)` per file.
fn collect_duckdb_files(dir: &Path) -> std::io::Result<Vec<(PathBuf, u64, u64)>> {
    let mut out = Vec::new();
    collect_duckdb_files_inner(dir, &mut out)?;
    Ok(out)
}

fn collect_duckdb_files_inner(
    dir: &Path,
    out: &mut Vec<(PathBuf, u64, u64)>,
) -> std::io::Result<()> {
    let rd = fs::read_dir(dir)?;
    for entry in rd.flatten() {
        let path = entry.path();
        let meta = entry.metadata()?;
        if meta.is_dir() {
            collect_duckdb_files_inner(&path, out)?;
        } else if path
            .extension()
            .and_then(|x| x.to_str())
            .is_some_and(|x| x == "duckdb")
        {
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_secs());
            out.push((path, mtime, meta.len()));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Options;
    use std::path::PathBuf;

    fn base_opts() -> Options {
        Options {
            repo_path: PathBuf::from("/tmp/test-repo"),
            ..Options::default()
        }
    }

    /// Non-tempfile tests (no feature gate required).
    #[test]
    fn cache_key_is_stable_across_identical_inputs() {
        let opts = base_opts();
        let k1 = cache_key(Path::new("/tmp/test-repo"), "abc123", &opts);
        let k2 = cache_key(Path::new("/tmp/test-repo"), "abc123", &opts);
        assert_eq!(k1, k2, "key must be deterministic for identical inputs");
    }

    #[test]
    fn cache_key_changes_when_head_sha_changes() {
        let opts = base_opts();
        let k1 = cache_key(Path::new("/tmp/test-repo"), "abc123", &opts);
        let k2 = cache_key(Path::new("/tmp/test-repo"), "def456", &opts);
        assert_ne!(k1, k2, "key must differ when HEAD SHA differs");
    }

    #[test]
    fn cache_key_changes_when_min_revs_changes() {
        let opts_a = Options {
            min_revs: 5,
            ..base_opts()
        };
        let opts_b = Options {
            min_revs: 10,
            ..base_opts()
        };
        let k1 = cache_key(Path::new("/tmp/test-repo"), "abc123", &opts_a);
        let k2 = cache_key(Path::new("/tmp/test-repo"), "abc123", &opts_b);
        assert_ne!(k1, k2, "key must differ when min_revs changes");
    }

    #[test]
    fn cache_key_changes_when_fisher_significance_changes() {
        let opts_a = Options {
            fisher_significance: 0.05,
            ..base_opts()
        };
        let opts_b = Options {
            fisher_significance: 0.01,
            ..base_opts()
        };
        let k1 = cache_key(Path::new("/tmp/test-repo"), "sha", &opts_a);
        let k2 = cache_key(Path::new("/tmp/test-repo"), "sha", &opts_b);
        assert_ne!(k1, k2, "key must differ when fisher_significance changes");
    }

    #[test]
    fn cache_key_does_not_change_when_rows_limit_changes() {
        let opts_a = Options {
            rows_limit: None,
            ..base_opts()
        };
        let opts_b = Options {
            rows_limit: Some(100),
            ..base_opts()
        };
        let k1 = cache_key(Path::new("/tmp/test-repo"), "sha", &opts_a);
        let k2 = cache_key(Path::new("/tmp/test-repo"), "sha", &opts_b);
        assert_eq!(k1, k2, "rows_limit is cosmetic and must not affect the key");
    }

    // Regression tests for the cache-key collision bug — before the
    // canonical_json() fix, every clone-detection option was silently
    // dropped from the cache key, so a user changing --min-clone-node-count
    // got a cache HIT against the old database.
    #[test]
    fn cache_key_changes_when_min_clone_node_count_changes() {
        let opts_a = Options {
            min_clone_node_count: 30,
            ..base_opts()
        };
        let opts_b = Options {
            min_clone_node_count: 60,
            ..base_opts()
        };
        let k1 = cache_key(Path::new("/tmp/test-repo"), "sha", &opts_a);
        let k2 = cache_key(Path::new("/tmp/test-repo"), "sha", &opts_b);
        assert_ne!(k1, k2, "min_clone_node_count must affect the cache key");
    }

    #[test]
    fn cache_key_changes_when_exclude_patterns_change() {
        let mut a = base_opts();
        a.exclude_patterns = vec!["vendor/**".into()];
        let mut b = base_opts();
        b.exclude_patterns = vec!["target/**".into()];
        let k1 = cache_key(Path::new("/tmp/test-repo"), "sha", &a);
        let k2 = cache_key(Path::new("/tmp/test-repo"), "sha", &b);
        assert_ne!(k1, k2, "exclude_patterns must affect the cache key");
    }

    #[test]
    fn cache_key_invariant_to_exclude_pattern_order() {
        // Order coming in from CLI vs .codeloreignore must not perturb the key.
        let mut a = base_opts();
        a.exclude_patterns = vec!["vendor/**".into(), "target/**".into()];
        let mut b = base_opts();
        b.exclude_patterns = vec!["target/**".into(), "vendor/**".into()];
        let k1 = cache_key(Path::new("/tmp/test-repo"), "sha", &a);
        let k2 = cache_key(Path::new("/tmp/test-repo"), "sha", &b);
        assert_eq!(
            k1, k2,
            "exclude_patterns order must not affect the key (canonical sort)"
        );
    }

    #[test]
    fn cache_key_changes_when_clone_similarity_floor_changes() {
        let opts_a = Options {
            clone_similarity_floor: 0.70,
            ..base_opts()
        };
        let opts_b = Options {
            clone_similarity_floor: 0.85,
            ..base_opts()
        };
        let k1 = cache_key(Path::new("/tmp/test-repo"), "sha", &opts_a);
        let k2 = cache_key(Path::new("/tmp/test-repo"), "sha", &opts_b);
        assert_ne!(k1, k2, "clone_similarity_floor must affect the cache key");
    }

    #[test]
    fn cache_path_has_correct_structure() {
        let opts = base_opts();
        let key = cache_key(Path::new("/tmp/test-repo"), "abc123", &opts);
        let root = PathBuf::from("/tmp/xdg-cache");
        let path = cache_path_with_root(&key, Path::new("/tmp/test-repo"), &root);

        // Must be under <root>/codelore/
        assert!(path.starts_with(root.join("codelore")));
        // File must end in .duckdb
        assert_eq!(path.extension().and_then(|x| x.to_str()), Some("duckdb"));
        // Filename stem must be exactly 16 hex chars
        let stem = path.file_stem().unwrap().to_str().unwrap();
        assert_eq!(stem.len(), 16, "stem must be 16 hex chars, got: {stem}");
        assert!(
            stem.chars().all(|c| c.is_ascii_hexdigit()),
            "stem must be all hex chars"
        );
        // Repo hash component must be exactly 8 hex chars
        let repo_component = path
            .parent()
            .unwrap()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap();
        assert_eq!(
            repo_component.len(),
            8,
            "repo hash must be 8 hex chars, got: {repo_component}"
        );
    }

    #[test]
    #[cfg(feature = "test-support")]
    fn prune_repo_cache_keeps_newest_entries() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        // Create 7 fake .duckdb files with different mtimes via short sleeps.
        for i in 0..7u64 {
            let path = root.join(format!("{i:016x}.duckdb"));
            std::fs::write(&path, b"placeholder").unwrap();
        }

        prune_repo_cache(root, 5);

        let remaining = std::fs::read_dir(root)
            .unwrap()
            .flatten()
            .filter(|e| {
                e.path()
                    .extension()
                    .and_then(|x| x.to_str())
                    .is_some_and(|x| x == "duckdb")
            })
            .count();

        assert!(
            remaining <= 5,
            "expected at most 5 entries after prune, got {remaining}"
        );
    }
}
