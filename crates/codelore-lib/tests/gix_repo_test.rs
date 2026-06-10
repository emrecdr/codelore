//! Integration tests for `GixRepo` against a tiny in-memory-style fixture repo.

use codelore_lib::repo::{GixRepo, Repo};
use codelore_lib::{ChangeType, Options};

#[test]
fn walks_tiny_repo_5_commits() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let opts = Options::default();
    let commits: Vec<_> = repo
        .walk_commits(&opts)
        .expect("walk")
        .map(|r| r.expect("commit"))
        .collect();
    assert_eq!(commits.len(), 5);
}

#[test]
fn changed_files_for_modify_commit() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let changes = repo.changed_files(&tiny.head_sha).expect("changed_files");
    // HEAD commit modifies src/main.rs only
    assert_eq!(changes.len(), 1);
    let c = &changes[0];
    assert_eq!(c.path, "src/main.rs");
    assert!(matches!(c.change_type, ChangeType::Modified));
}

/// F34 regression — a commit that adds a small binary blob (containing
/// NUL bytes in the first 8 KB) must report `loc_added = 0` and
/// `loc_deleted = 0`, mirroring `GitCliRepo` (which sees `- -` for
/// binary files from `git log --numstat`). Pre-F34, `count_loc` ran
/// imara-diff over arbitrary byte arrays and produced nonsense
/// "additions" / "removals" counts.
#[test]
fn binary_blob_reports_zero_loc_under_gix() {
    use std::process::Command;
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path();
    let git = |args: &[&str]| {
        let ok = Command::new("git")
            .arg("-C")
            .arg(path)
            .args(args)
            .status()
            .expect("spawn git")
            .success();
        assert!(ok, "git {args:?} failed");
    };
    git(&["init", "-b", "main", "--quiet"]);
    git(&["config", "user.email", "f34@example.com"]);
    git(&["config", "user.name", "F34"]);

    std::fs::write(path.join("readme.txt"), "hello\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-m", "seed", "--quiet"]);

    // Synthetic binary: 256 bytes with NUL bytes scattered through the
    // first 8 KB window. Git itself would classify this as binary
    // (`git diff --numstat` returns `- -`).
    let mut bin = Vec::with_capacity(256);
    for i in 0..256u32 {
        #[allow(clippy::cast_possible_truncation)]
        bin.push(i as u8);
    }
    std::fs::write(path.join("blob.bin"), &bin).unwrap();
    git(&["add", "."]);
    git(&["commit", "-m", "add binary", "--quiet"]);

    let repo = GixRepo::open(path).expect("open");
    let opts = Options::default();
    let commits: Vec<_> = repo
        .walk_commits(&opts)
        .expect("walk")
        .map(|r| r.expect("commit"))
        .collect();
    let head_changes = &commits[0].changes;
    let blob = head_changes
        .iter()
        .find(|c| c.path == "blob.bin")
        .expect("blob.bin must appear in HEAD commit");
    assert_eq!(
        (blob.loc_added, blob.loc_deleted),
        (0, 0),
        "F34: binary blob (NUL bytes in first 8KB) must report \
         loc_added=0, loc_deleted=0 (git's `- -` convention). Got: \
         loc_added={}, loc_deleted={}",
        blob.loc_added,
        blob.loc_deleted
    );
}
