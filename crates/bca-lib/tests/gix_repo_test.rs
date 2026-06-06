//! Integration tests for `GixRepo` against a tiny in-memory-style fixture repo.

use bca_lib::repo::{GixRepo, Repo};
use bca_lib::{ChangeType, Options};

#[test]
fn walks_tiny_repo_5_commits() {
    let tiny = bca_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let opts = Options::default();
    let commits: Vec<_> = repo.walk_commits(&opts).expect("walk").collect();
    assert_eq!(commits.len(), 5);
}

#[test]
fn changed_files_for_modify_commit() {
    let tiny = bca_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let changes = repo.changed_files(&tiny.head_sha).expect("changed_files");
    // HEAD commit modifies src/main.rs only
    assert_eq!(changes.len(), 1);
    let c = &changes[0];
    assert_eq!(c.path, "src/main.rs");
    assert!(matches!(c.change_type, ChangeType::Modified));
}
