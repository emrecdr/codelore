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

/// A commit that adds a small binary blob (containing
/// NUL bytes in the first 8 KB) must report `loc_added = 0` and
/// `loc_deleted = 0`, mirroring `GitCliRepo` (which sees `- -` for
/// binary files from `git log --numstat`). Previously, `count_loc` ran
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
    git(&["config", "user.email", "binary-fixture@example.com"]);
    git(&["config", "user.name", "Binary Fixture"]);

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
        "binary blob (NUL bytes in first 8KB) must report \
         loc_added=0, loc_deleted=0 (git's `- -` convention). Got: \
         loc_added={}, loc_deleted={}",
        blob.loc_added,
        blob.loc_deleted
    );
}

/// `git clone --depth=1` a `source` repo (a local path) into a fresh tempdir.
///
/// `--depth` on a *local path* clone source silently degrades to git's
/// default hardlink-based local-clone optimization and ignores the flag
/// entirely — `git` prints `--depth is ignored in local clones; use file://
/// instead` on stderr and produces a full, non-shallow clone. A genuine
/// shallow clone from a local fixture therefore requires the `file://` URL
/// form, which forces the real transport-level clone path `--depth` needs
/// (verified empirically against the git version this suite runs under).
fn shallow_clone(source: &std::path::Path) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let source_url = format!("file://{}", source.display());
    let status = std::process::Command::new("git")
        .args(["clone", "--quiet", "--depth=1"])
        .arg(&source_url)
        .arg(dir.path())
        .status()
        .expect("spawn git clone --depth=1");
    assert!(
        status.success(),
        "shallow git clone from {source_url} failed"
    );
    dir
}

#[test]
fn is_shallow_true_for_a_depth_one_clone() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let shallow = shallow_clone(tiny.dir.path());
    let repo = GixRepo::open(shallow.path()).expect("open shallow clone");
    assert!(
        repo.is_shallow(),
        "a --depth=1 clone must report is_shallow() == true"
    );
}

#[test]
fn is_shallow_false_for_a_full_clone() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open full clone");
    assert!(
        !repo.is_shallow(),
        "a full (non-shallow) clone must report is_shallow() == false"
    );
}
