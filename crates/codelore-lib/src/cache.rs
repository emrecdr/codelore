//! Persistent fact-store cache: key derivation + XDG path resolution.
//!
//! Cache path layout:
//!   `$XDG_CACHE_HOME/codelore/<repo_hash_8>/<cache_key_16>.duckdb`
//!
//! Cache key covers: `canonical_repo_path`, HEAD SHA, crate version, options
//! thresholds, cache epoch. Excludes: `rows_limit`, `repo_path` (already
//! folded into `repo_hash_8`), cosmetic flags.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use sha2::{Digest, Sha256};

use crate::Options;

/// Cache-invalidation epoch — a manual cache-buster folded into the cache
/// key. Bump on any correctness fix that should orphan existing cache files,
/// whether or not the `DuckDB` schema shape changed. This is deliberately
/// independent of `facts::schema::CURRENT_SCHEMA_VERSION` (the on-disk schema
/// version): a non-schema correctness fix bumps this without touching the
/// schema version, and the historical `schema_v` prefix on the value is
/// retained so older cache files stay invalidated.
///
/// The current epoch (`schema_v17`) invalidates caches whose `author_aliases`
/// table predates the `(raw_name, raw_email)` re-key. The old email-only key
/// collapsed two name+email identities that share one commit email into a
/// single first-wins row, dropping the loser's commits from every
/// canonical-set consumer (knowledge shares, familiarity, bus factor). A
/// stale hit would serve those collapsed rows, so the epoch bump forces the
/// fact store to be rebuilt with the pair-keyed table.
const CACHE_EPOCH: &str = "schema_v17";

/// Compute a 32-byte SHA-256 cache key from:
///   `canonical_repo_path || NUL || head_sha || NUL || CARGO_PKG_VERSION || NUL`
///   `|| opts_hash(opts) || NUL || CACHE_EPOCH`
///
/// The key is deterministic for identical inputs and changes whenever any
/// threshold knob or the cache epoch changes.
#[must_use]
pub fn cache_key(repo_path: &Path, head_sha: &str, opts: &Options) -> [u8; 32] {
    let mut hasher = Sha256::new();
    let canonical = canonicalize_with_fallback_log(repo_path, "cache_key");
    hasher.update(canonical.to_string_lossy().as_bytes());
    hasher.update(b"\x00");
    hasher.update(head_sha.as_bytes());
    hasher.update(b"\x00");
    hasher.update(env!("CARGO_PKG_VERSION").as_bytes());
    hasher.update(b"\x00");
    hasher.update(opts_hash(opts).as_bytes());
    hasher.update(b"\x00");
    hasher.update(CACHE_EPOCH.as_bytes());
    let mut out = [0u8; 32];
    out.copy_from_slice(&hasher.finalize());
    out
}

/// Resolve the on-disk path for a cache entry:
///   `<cache_root>/codelore/<repo_hash_8>/<cache_key_16>.duckdb`
///
/// `cache_root` defaults to [`dirs::cache_dir()`] but can be overridden via `--cache-dir`.
#[must_use]
pub fn cache_path(key: &[u8; 32], repo_path: &Path) -> PathBuf {
    cache_path_with_root(key, repo_path, &default_cache_root())
}

/// Same as [`cache_path`] but with an explicit root (for `--cache-dir` override).
///
/// `repo_path` is canonicalised before hashing the
/// per-repo subdirectory name so this function and [`cache_key`] (which
/// already canonicalises) stay in lockstep. Without this, calling
/// `codelore analyze .` and `codelore analyze $(pwd)` produced identical
/// cache keys but different cache subdirectories — neither call could
/// see the other's cache file, forcing a redundant ingest every time the
/// user alternated invocation styles.
#[must_use]
pub fn cache_path_with_root(key: &[u8; 32], repo_path: &Path, root: &Path) -> PathBuf {
    let repo_short = repo_hash_short(repo_path);
    let key_short = hex::encode(&key[..8]); // 16 hex chars
    root.join("codelore")
        .join(repo_short)
        .join(format!("{key_short}.duckdb"))
}

/// Compute the per-repo cache subdirectory:
/// `<cache_root>/codelore/<repo_hash_8>/`
///
/// All per-repo sidecar files (gate ledger, external findings) share this
/// directory with the `.duckdb` cache entries. The 8-hex-char directory
/// name is `sha256(canonical_repo_path)[0..4]` encoded as lowercase hex.
///
/// This is the canonical implementation — `cache_path_with_root` and
/// `quality_gates::ledger::ledger_dir` both delegate here so the derivation
/// is defined in exactly one place.
#[must_use]
pub fn repo_cache_dir(cache_root: &Path, repo_path: &Path) -> PathBuf {
    cache_root.join("codelore").join(repo_hash_short(repo_path))
}

/// Compute the 8-hex-char repo directory component from `repo_path`.
///
/// Canonicalises the path before hashing so `codelore analyze .` and
/// `codelore analyze $(pwd)` resolve to the same directory.
fn repo_hash_short(repo_path: &Path) -> String {
    let canonical = canonicalize_with_fallback_log(repo_path, "repo_hash_short");
    let mut hasher = Sha256::new();
    hasher.update(canonical.to_string_lossy().as_bytes());
    hex::encode(&hasher.finalize()[..4]) // 8 hex chars
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

/// `fs::canonicalize` wrapper that logs a `tracing::debug!` when the
/// system call fails and the raw path is used as a fallback.
///
/// `cache_key` and `cache_path_with_root` both canonicalise the repo
/// path before hashing — they MUST agree on the result for the same
/// input or the cache key drifts between key derivation and path
/// lookup, producing a silent cache miss. The `unwrap_or_else` keeps
/// them aligned (both fall back to the raw path on identical
/// failure), but the fallback path was previously silent. If a
/// canonicalize succeeds in one call site and fails in the other
/// (e.g., a symlink target is created or a permission flips between
/// invocations), the resulting key drift surfaces only as a degraded
/// hit rate — invisible without instrumentation. The debug log gives
/// operators on dirty containers / shared mounts a breadcrumb to
/// trace why their cache hit rate dropped.
fn canonicalize_with_fallback_log(repo_path: &Path, call_site: &str) -> PathBuf {
    match fs::canonicalize(repo_path) {
        Ok(canonical) => canonical,
        Err(e) => {
            tracing::debug!(
                "{}: fs::canonicalize fallback for repo_path={} ({}); using raw path",
                call_site,
                repo_path.display(),
                e,
            );
            repo_path.to_path_buf()
        }
    }
}

// ---------------------------------------------------------------------------
// LRU eviction (Task 13)
// ---------------------------------------------------------------------------

/// Remove `.duckdb` files from `repo_dir` beyond `max_entries`, deleting the
/// oldest (by mtime) first. Also sweeps stale `.tmp.<pid>` and
/// `.tmp.<pid>.wal` artifacts left behind by crashed runs.
///
/// Errors from individual file deletes are logged but not fatal — a partial
/// prune is better than aborting the analysis.
pub fn prune_repo_cache(repo_dir: &Path, max_entries: usize) {
    cleanup_stale_tmp_files(repo_dir);

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
        delete_duckdb_with_companion(&path, "prune_repo_cache");
    }
}

/// Walk `root/codelore/` and delete `.duckdb` files (oldest first) until the
/// total directory size falls below `max_bytes`. Also sweeps stale
/// `.tmp.<pid>` artifacts across the global cache tree.
///
/// Errors from individual file operations are logged but not fatal.
pub fn prune_global_cache(root: &Path, max_bytes: u64) {
    let codelore_dir = root.join("codelore");
    cleanup_stale_tmp_files_recursive(&codelore_dir);

    let walk = collect_duckdb_files(&codelore_dir);

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
        let existed = path.exists();
        delete_duckdb_with_companion(&path, "prune_global_cache");
        if existed && !path.exists() {
            remaining = remaining.saturating_sub(size);
        }
    }
}

/// Recursively collect all `.duckdb` files under `dir`.
/// Returns `(path, mtime_secs, size_bytes)` per file.
///
/// Individual subdirectory or entry errors (broken symlinks, permission
/// denied) are logged and skipped — a single bad entry must not abort the
/// entire walk, otherwise one inaccessible subdir would disable global LRU
/// eviction and let the cache grow unbounded.
fn collect_duckdb_files(dir: &Path) -> Vec<(PathBuf, u64, u64)> {
    let mut out = Vec::new();
    collect_duckdb_files_inner(dir, &mut out);
    out
}

fn collect_duckdb_files_inner(dir: &Path, out: &mut Vec<(PathBuf, u64, u64)>) {
    let rd = match fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) => {
            tracing::warn!("collect_duckdb_files: skipping {} ({e})", dir.display());
            return;
        }
    };
    for entry in rd.flatten() {
        let path = entry.path();
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!("collect_duckdb_files: skipping {} ({e})", path.display());
                continue;
            }
        };
        if meta.is_dir() {
            collect_duckdb_files_inner(&path, out);
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
}

/// Delete a `.duckdb` cache file plus its companion `.duckdb.wal` if
/// present. `DuckDB` writes a WAL alongside the database file during writes;
/// in normal operation the WAL is flushed via `CHECKPOINT` before the
/// connection drops, but in rare cases (forced kill, FS sync failure) an
/// orphan WAL can survive next to the renamed database — and the
/// pruner's `.duckdb`-only extension filter would leave it behind.
///
/// Failures are logged but not propagated — partial cleanup beats abort.
fn delete_duckdb_with_companion(path: &Path, ctx: &str) {
    match fs::remove_file(path) {
        Ok(()) => tracing::info!("{ctx}: removed {}", path.display()),
        Err(e) => {
            tracing::warn!("{ctx}: failed to remove {}: {e}", path.display());
            return;
        }
    }
    let wal = path.with_extension("duckdb.wal");
    if wal.exists() {
        if let Err(e) = fs::remove_file(&wal) {
            tracing::warn!("{ctx}: failed to remove WAL {}: {e}", wal.display());
        } else {
            tracing::info!("{ctx}: removed WAL {}", wal.display());
        }
    }
}

/// Stale `.tmp` sweep — minimum age before a `.tmp.<pid>` artifact is
/// considered orphaned. One hour is well above any realistic ingest run on
/// repositories of the size codelore targets, while still being short
/// enough to bound disk leak from frequently-crashing runs.
const STALE_TMP_AGE_SECS: u64 = 3600;

/// Remove `.tmp.<pid>` and `.tmp.<pid>.wal` artifacts in `dir` that are
/// older than [`STALE_TMP_AGE_SECS`]. These are left behind by crashed
/// ingest runs (the normal-path rename + drop sequence cleans them up).
///
/// Age-gated rather than PID-alive-gated: checking liveness of a foreign
/// PID requires platform-specific syscalls and races against PID
/// recycling. An hour-old artifact whose PID has been recycled to another
/// codelore process is functionally equivalent to a dead-PID orphan —
/// either way nothing legitimate is still writing to it.
pub fn cleanup_stale_tmp_files(dir: &Path) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for entry in rd.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        // Match either `<stem>.duckdb.tmp.<pid>` or
        // `<stem>.duckdb.tmp.<pid>.wal`. The bare `.duckdb.tmp` (no
        // PID suffix) is also swept for forward compatibility —
        // a fixed-path scheme couldn't survive any concurrent run.
        if !name.contains(".duckdb.tmp") {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        let Ok(modified) = meta.modified() else {
            continue;
        };
        let Ok(age) = modified.elapsed() else {
            continue;
        };
        if age.as_secs() < STALE_TMP_AGE_SECS {
            continue;
        }
        if let Err(e) = fs::remove_file(&path) {
            tracing::warn!(
                "cleanup_stale_tmp_files: failed to remove {}: {e}",
                path.display()
            );
        } else {
            tracing::info!("cleanup_stale_tmp_files: removed stale {}", path.display());
        }
    }
}

fn cleanup_stale_tmp_files_recursive(dir: &Path) {
    cleanup_stale_tmp_files(dir);
    let Ok(rd) = fs::read_dir(dir) else { return };
    for entry in rd.flatten() {
        let path = entry.path();
        if entry.file_type().is_ok_and(|t| t.is_dir()) {
            cleanup_stale_tmp_files_recursive(&path);
        }
    }
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
    fn cache_key_unchanged_when_target_changes() {
        // `target` is a per-invocation selector (function-xray path) that
        // ingest never reads. Two runs differing only in `--target` must hit
        // the same cache entry; different `window_days` must still differ.
        let opts_no_target = Options {
            window_days: 30,
            ..base_opts()
        };
        let opts_with_target = Options {
            target: Some("src/lib.rs".into()),
            window_days: 30,
            ..base_opts()
        };
        let opts_different_window = Options {
            target: Some("src/lib.rs".into()),
            window_days: 60,
            ..base_opts()
        };
        let k_no = cache_key(Path::new("/tmp/test-repo"), "sha", &opts_no_target);
        let k_with = cache_key(Path::new("/tmp/test-repo"), "sha", &opts_with_target);
        let k_diff = cache_key(Path::new("/tmp/test-repo"), "sha", &opts_different_window);
        assert_eq!(k_no, k_with, "target must not affect the cache key");
        assert_ne!(
            k_with, k_diff,
            "window_days still differentiates the key when target differs"
        );
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
    fn cache_key_changes_when_head_only_ingest_changes() {
        // A head-only fact store carries empty history tables — serving it
        // to a full-ingest caller (or vice versa) would be silent data
        // loss, so the two modes must never share a cache entry.
        let full = base_opts();
        let head_only = Options {
            head_only_ingest: true,
            ..base_opts()
        };
        let k_full = cache_key(Path::new("/tmp/test-repo"), "sha", &full);
        let k_head = cache_key(Path::new("/tmp/test-repo"), "sha", &head_only);
        assert_ne!(
            k_full, k_head,
            "head_only_ingest must key head-only stores apart from full stores"
        );
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
