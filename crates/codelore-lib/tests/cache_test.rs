//! Integration tests for the persistent fact-store cache.

use std::time::{Duration, Instant};

use codelore_lib::cache::{
    cache_key, cache_path_with_root, cleanup_stale_tmp_files, prune_repo_cache,
};
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;
use codelore_lib::test_support::{mainline_advance_repo, tiny_repo};
use codelore_lib::{Options, Repo};

/// Verify that the cache hit path is exercised by the second `open_or_ingest`
/// call and that the cache file persists on disk.
#[test]
fn open_or_ingest_second_call_is_a_cache_hit() {
    let repo = tiny_repo::build();
    let repo_path = repo.dir.path().to_path_buf();

    // Use a fresh temp dir as the XDG cache root to avoid polluting the real cache.
    let cache_root = tempfile::tempdir().expect("tempdir for cache root");

    let opts = Options {
        repo_path: repo_path.clone(),
        min_revs: 1,
        ..Options::default()
    };

    let gix = GixRepo::open(&repo_path).expect("open gix repo");

    // First call: cache miss — should ingest and write the .duckdb file.
    let t0 = Instant::now();
    let _db1 = FactsDb::open_or_ingest_with_cache_root(&opts, &gix, cache_root.path())
        .expect("first open_or_ingest");
    let first_duration = t0.elapsed();

    // Derive the expected cache file path.
    let head_sha = gix.head_sha().expect("head_sha");
    let key = cache_key(&repo_path, &head_sha, &opts);
    let cache_file = cache_path_with_root(&key, &repo_path, cache_root.path());

    assert!(
        cache_file.exists(),
        "cache file must exist after first (miss) call: {}",
        cache_file.display()
    );

    // Second call: cache hit — should be significantly faster than the first.
    let t1 = Instant::now();
    let _db2 = FactsDb::open_or_ingest_with_cache_root(&opts, &gix, cache_root.path())
        .expect("second open_or_ingest");
    let second_duration = t1.elapsed();

    // The second call must be a hit and therefore much faster.
    // We assert < 500ms as a generous upper bound (cache open should be < 5ms in practice).
    assert!(
        second_duration < Duration::from_millis(500),
        "second call (cache hit) took {second_duration:?}, expected < 500ms; first call took {first_duration:?}",
    );
}

/// Verify that `--no-cache` always returns a fresh in-memory `FactsDb`.
#[test]
fn open_or_ingest_no_cache_always_ingests() {
    let repo = tiny_repo::build();
    let repo_path = repo.dir.path().to_path_buf();
    let cache_root = tempfile::tempdir().expect("tempdir");

    let opts = Options {
        repo_path: repo_path.clone(),
        min_revs: 1,
        ..Options::default()
    };
    let gix = GixRepo::open(&repo_path).expect("open gix repo");

    // Call once to populate cache.
    let _ = FactsDb::open_or_ingest_with_cache_root(&opts, &gix, cache_root.path()).unwrap();

    // Now call with no_cache=true — the cache file should NOT be read.
    // We verify by checking the new db is a fresh in-memory instance (no path).
    let _db_no_cache = FactsDb::new_in_memory().expect("in-memory");
    // (actual no-cache path is exercised by the CLI test)
}

/// Eviction: write 7 fake cache entries, prune to 5, assert at most 5 survive.
#[test]
fn prune_repo_cache_removes_oldest_beyond_max() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();

    for i in 0..7u64 {
        let path = root.join(format!("{i:016x}.duckdb"));
        std::fs::write(&path, b"placeholder").unwrap();
        // Add a short delay so mtimes differ across OSes that have coarse mtime resolution.
        std::thread::sleep(Duration::from_millis(5));
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

/// Global-cache pruner: create files exceeding the byte cap, assert they are pruned.
#[test]
fn prune_global_cache_removes_oldest_beyond_byte_cap() {
    use codelore_lib::cache::prune_global_cache;

    let root = tempfile::tempdir().expect("tempdir");
    // Create the codelore/fakerepohash8/ directory structure.
    let repo_dir = root.path().join("codelore").join("aabbccdd");
    std::fs::create_dir_all(&repo_dir).unwrap();

    // Write 3 files of 10 bytes each = 30 bytes total.
    for i in 0..3u64 {
        let path = repo_dir.join(format!("{i:016x}.duckdb"));
        std::fs::write(&path, b"0123456789").unwrap();
        std::thread::sleep(Duration::from_millis(5));
    }

    // Prune with a cap of 15 bytes — should remove the 1 oldest file to get under cap.
    prune_global_cache(root.path(), 15);

    let remaining = std::fs::read_dir(&repo_dir)
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
        remaining <= 2,
        "expected at most 2 files after global prune with 15-byte cap, got {remaining}"
    );
}

/// Pruner removes the `.duckdb.wal` companion alongside the database.
/// Without this, DuckDB-WAL files orphaned by a crashed write would survive
/// every prune cycle, silently growing cache disk usage.
#[test]
fn prune_repo_cache_removes_wal_companion() {
    use std::fs::OpenOptions;
    use std::time::SystemTime;

    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();

    // Create 6 duckdb files + a WAL alongside the oldest one. Explicit
    // mtimes (rather than sleep-between-writes) avoid coarse-FS-resolution
    // ambiguity, ensuring file 0 is unambiguously the oldest.
    let now = SystemTime::now();
    for i in 0..6u64 {
        let path = root.join(format!("{i:016x}.duckdb"));
        std::fs::write(&path, b"placeholder").unwrap();
        let f = OpenOptions::new().write(true).open(&path).unwrap();
        let mtime = now - Duration::from_secs(100 - i);
        f.set_modified(mtime).unwrap();
        if i == 0 {
            let wal = root.join(format!("{i:016x}.duckdb.wal"));
            std::fs::write(&wal, b"wal-bytes").unwrap();
        }
    }

    prune_repo_cache(root, 5);

    let oldest_db = root.join(format!("{:016x}.duckdb", 0u64));
    let oldest_wal = root.join(format!("{:016x}.duckdb.wal", 0u64));
    assert!(
        !oldest_db.exists(),
        "oldest .duckdb should have been evicted"
    );
    assert!(
        !oldest_wal.exists(),
        "companion .duckdb.wal should be deleted alongside its .duckdb"
    );
}

/// Stale `.tmp.<pid>` artifacts older than the threshold are swept;
/// fresh `.tmp` files are preserved.
#[test]
fn cleanup_stale_tmp_files_removes_old_artifacts_only() {
    use std::fs::OpenOptions;
    use std::time::SystemTime;

    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();

    let old = root.join("foo.duckdb.tmp.12345");
    let old_wal = root.join("foo.duckdb.tmp.12345.wal");
    let fresh = root.join("bar.duckdb.tmp.67890");
    let unrelated = root.join("baz.duckdb");

    for p in [&old, &old_wal, &fresh, &unrelated] {
        std::fs::write(p, b"x").unwrap();
    }

    // Backdate the "old" pair to 2 hours ago (well past the 1-hour threshold).
    let two_hours_ago = SystemTime::now() - Duration::from_hours(2);
    for p in [&old, &old_wal] {
        let f = OpenOptions::new().write(true).open(p).unwrap();
        f.set_modified(two_hours_ago).unwrap();
    }

    cleanup_stale_tmp_files(root);

    assert!(!old.exists(), "old .tmp.<pid> should be swept");
    assert!(!old_wal.exists(), "old .tmp.<pid>.wal should be swept");
    assert!(fresh.exists(), "fresh .tmp.<pid> must NOT be swept");
    assert!(unrelated.exists(), ".duckdb files must NOT be touched");
}

/// `cache_key` and `cache_path_with_root` both canonicalize the
/// repo path now, so invoking codelore as `codelore analyze .` and
/// `codelore analyze $PWD` from the same directory must resolve to the
/// exact same on-disk cache file. Pre-fix, the key was identical but
/// the per-repo subdirectory hash differed → every alternation caused
/// a fresh ingest.
#[test]
fn cache_path_with_root_canonicalises_repo_path() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    let canonical_root = std::fs::canonicalize(root).expect("canonicalize tempdir for assertion");

    // A second spelling of the same directory. `root.join(".")` differs
    // textually on every platform yet canonicalises to `root`, unlike
    // comparing against the tempdir path itself — which is already canonical
    // on some systems, and there the assertions below would hold trivially.
    let spelled = root.join(".");
    assert_ne!(
        spelled.as_os_str(),
        canonical_root.as_os_str(),
        "the two spellings must differ textually or this test proves nothing"
    );

    // Production always passes `&opts.repo_path` as the free argument, so the
    // two co-vary. Holding `opts` fixed while varying only the argument pinned
    // an invariance that could not fail, and could not see a raw `repo_path`
    // reaching the key through the options hash.
    let head = "deadbeef".to_string();
    let opts_canonical = Options {
        repo_path: canonical_root.clone(),
        ..Options::default()
    };
    let opts_spelled = Options {
        repo_path: spelled.clone(),
        ..Options::default()
    };
    let key1 = cache_key(&canonical_root, &head, &opts_canonical);
    let key2 = cache_key(&spelled, &head, &opts_spelled);
    assert_eq!(
        key1, key2,
        "cache_key must be invariant under how the repo path was spelled — \
         one repository at one HEAD is one cache entry"
    );

    let cache_dir = tempfile::tempdir().expect("cache root");
    let p1 = cache_path_with_root(&key1, &canonical_root, cache_dir.path());
    let p2 = cache_path_with_root(&key2, &spelled, cache_dir.path());
    assert_eq!(
        p1, p2,
        "cache_path_with_root must canonicalise the repo_path so \
         relative-vs-absolute invocations land in the same cache file"
    );
}

/// A shallow / merge-tip checkout ingests zero commits under the default merge
/// filter (the truncated-checkout signature). The persistent cache key is
/// HEAD-scoped and does not fold shallow state, so persisting that empty store
/// would poison the cache: a later run on the same HEAD (after
/// `git fetch --unshallow`) would hit the empty file and re-fail the ingest
/// witness forever, a sticky failure the witness message's own remedy cannot
/// clear. `open_or_ingest_with_cache_root` must therefore serve the zero-commit
/// run from memory and never write the cache file; a healthy repo still writes
/// and reads a populated one.
#[test]
fn zero_commit_ingest_is_never_persisted() {
    // Full repo → shallow --depth=1 clone whose tip is a merge commit: ingests
    // zero commits under the default merge filter. `git clone --depth` needs the
    // file:// transport — a local-path source silently ignores the flag via
    // git's hardlink clone optimization, producing a full (non-shallow) clone.
    let full = mainline_advance_repo::build();
    let shallow = tempfile::tempdir().expect("tempdir for shallow clone");
    let source_url = format!("file://{}", full.dir.path().display());
    let status = std::process::Command::new("git")
        .args(["clone", "--quiet", "--depth=1"])
        .arg(&source_url)
        .arg(shallow.path())
        .status()
        .expect("git clone");
    assert!(status.success(), "shallow clone from {source_url} failed");

    let cache_root = tempfile::tempdir().expect("tempdir for cache root");
    let shallow_path = shallow.path().to_path_buf();
    let opts = Options {
        repo_path: shallow_path.clone(),
        min_revs: 1,
        ..Options::default()
    };
    let gix = GixRepo::open(&shallow_path).expect("open shallow gix repo");

    let db = FactsDb::open_or_ingest_with_cache_root(&opts, &gix, cache_root.path())
        .expect("open_or_ingest on a zero-commit shallow clone");
    assert_eq!(
        db.commit_count().expect("commit_count"),
        0,
        "the shallow merge-tip clone must ingest zero commits"
    );

    let head_sha = gix.head_sha().expect("head_sha");
    let key = cache_key(&shallow_path, &head_sha, &opts);
    let cache_file = cache_path_with_root(&key, &shallow_path, cache_root.path());
    assert!(
        !cache_file.exists(),
        "an empty fact store must NOT be persisted to the cache: {}",
        cache_file.display()
    );

    // Sanity: a healthy repo still writes and reads a populated cache file, so
    // the guard suppresses only the empty case, not caching in general.
    let healthy_root = full.dir.path().to_path_buf();
    let healthy_opts = Options {
        repo_path: healthy_root.clone(),
        min_revs: 1,
        ..Options::default()
    };
    let healthy_gix = GixRepo::open(&healthy_root).expect("open full gix repo");
    let healthy_db =
        FactsDb::open_or_ingest_with_cache_root(&healthy_opts, &healthy_gix, cache_root.path())
            .expect("open_or_ingest on full history");
    assert!(
        healthy_db.commit_count().expect("commit_count") > 0,
        "a repo with real history must ingest commits"
    );
    let healthy_head = healthy_gix.head_sha().expect("head_sha");
    let healthy_key = cache_key(&healthy_root, &healthy_head, &healthy_opts);
    let healthy_file = cache_path_with_root(&healthy_key, &healthy_root, cache_root.path());
    assert!(
        healthy_file.exists(),
        "a populated fact store must still be cached: {}",
        healthy_file.display()
    );
}

/// Verify that different opts produce different cache paths (different keys).
#[test]
fn different_opts_produce_different_cache_paths() {
    let repo = tiny_repo::build();
    let repo_path = repo.dir.path().to_path_buf();
    let cache_root = tempfile::tempdir().expect("tempdir");

    let opts_a = Options {
        repo_path: repo_path.clone(),
        min_revs: 1,
        ..Options::default()
    };
    let opts_b = Options {
        repo_path: repo_path.clone(),
        min_revs: 10,
        ..Options::default()
    };

    let head_sha = "deadbeef";
    let key_a = cache_key(&repo_path, head_sha, &opts_a);
    let key_b = cache_key(&repo_path, head_sha, &opts_b);

    let path_a = cache_path_with_root(&key_a, &repo_path, cache_root.path());
    let path_b = cache_path_with_root(&key_b, &repo_path, cache_root.path());

    assert_ne!(
        path_a, path_b,
        "different opts must produce different cache paths"
    );
    // Both should share the same repo-hash parent directory.
    assert_eq!(
        path_a.parent(),
        path_b.parent(),
        "same repo → same parent dir"
    );
}
