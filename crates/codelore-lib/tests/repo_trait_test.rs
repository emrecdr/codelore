//! Compile-time test: confirms the `Repo` trait shape exists.
//!
//! Behavioural coverage lives in `gix_repo_test.rs`, `git_cli_repo_test.rs`,
//! and `differential_repo_test.rs`, which cross-checks the two backends
//! against each other over a fixture repository.

use codelore_lib::CommitEvent;
use codelore_lib::Options;
use codelore_lib::repo::{GixRepo, Repo};

#[test]
fn gix_repo_walks_self_repo() {
    // CARGO_MANIFEST_DIR points to the crate root; the git repo is two levels up.
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir.parent().unwrap().parent().unwrap();
    let repo = GixRepo::open(repo_root).expect("open self repo");
    let opts = Options::default();
    let commits: Vec<_> = repo
        .walk_commits(&opts)
        .expect("walk")
        .map(|r| r.expect("commit")) // surfaces any per-commit errors
        .collect();
    assert!(
        !commits.is_empty(),
        "expected at least one commit in the test repo"
    );
}

#[allow(
    dead_code,
    unreachable_code,
    unused_variables,
    clippy::diverging_sub_expression
)]
fn _trait_object_compiles<R: Repo>(r: &R) {
    let _: Box<dyn Iterator<Item = codelore_lib::Result<CommitEvent>>> = unimplemented!();
    let opts = Options::default();
    let _: codelore_lib::Result<
        Box<dyn Iterator<Item = codelore_lib::Result<CommitEvent>> + Send + '_>,
    > = r.walk_commits(&opts);
    let _: codelore_lib::Result<Vec<codelore_lib::FileChange>> = r.changed_files("abc");
    let _: codelore_lib::Result<Vec<codelore_lib::Hunk>> = r.diff_hunks("abc", "src/main.rs");
    let _: String = r.resolve_alias("Some Name", "a@b.com");
}
