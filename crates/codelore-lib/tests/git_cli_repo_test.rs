//! Integration tests for `GitCliRepo` — the shell-out Repo impl used for differential testing.

use codelore_lib::Options;
use codelore_lib::repo::{GitCliRepo, Repo};

#[test]
fn git_cli_repo_opens_tiny_repo() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let _repo = GitCliRepo::open(tiny.dir.path()).expect("open");
}

#[test]
fn git_cli_repo_rejects_non_repo() {
    let dir = tempfile::tempdir().expect("tempdir");
    let result = GitCliRepo::open(dir.path());
    assert!(result.is_err(), "non-repo should fail to open");
}

#[test]
fn git_cli_repo_walks_tiny_repo() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GitCliRepo::open(tiny.dir.path()).expect("open");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        ..Options::default()
    };
    let events: Vec<_> = repo
        .walk_commits(&opts)
        .expect("walk")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect");
    assert!(
        events.len() >= 5,
        "tiny_repo has 5 commits, got {}",
        events.len()
    );
}

#[test]
fn git_cli_repo_walk_commits_have_required_fields() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GitCliRepo::open(tiny.dir.path()).expect("open");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        ..Options::default()
    };
    let events: Vec<_> = repo
        .walk_commits(&opts)
        .expect("walk")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect");

    let head = &events[0];
    // tiny_repo configures user.email = tiny@example.com
    assert_eq!(head.author_email, "tiny@example.com");
    assert_eq!(head.author_name, "Tiny");
    assert!(!head.rev.is_empty(), "rev should be non-empty");
    assert!(!head.message.is_empty(), "message should be non-empty");
}

#[test]
fn git_cli_repo_resolve_alias_returns_input_on_no_match() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GitCliRepo::open(tiny.dir.path()).expect("open");
    let resolved = repo.resolve_alias("", "unmapped@example.com");
    assert_eq!(resolved, "unmapped@example.com");
}

#[test]
fn git_cli_repo_resolve_alias_with_mailmap() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path();

    run_git(path, &["init", "-b", "main", "--quiet"]);
    run_git(path, &["config", "user.email", "alice@old.com"]);
    run_git(path, &["config", "user.name", "Alice"]);

    std::fs::write(
        path.join(".mailmap"),
        "Alice Real <alice@real.com> <alice@old.com>\n",
    )
    .unwrap();
    std::fs::write(path.join("README.md"), "hello\n").unwrap();
    run_git(path, &["add", "."]);
    run_git(path, &["commit", "-m", "init", "--quiet"]);

    let repo = GitCliRepo::open(path).expect("open");
    let canonical = repo.resolve_alias("", "alice@old.com");
    assert_eq!(canonical, "alice@real.com", "mailmap should map old → real");
}

#[test]
fn git_cli_repo_lists_changed_files() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GitCliRepo::open(tiny.dir.path()).expect("open");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        ..Options::default()
    };
    let first = repo.walk_commits(&opts).unwrap().next().unwrap().unwrap();
    let files = repo.changed_files(&first.rev).expect("changed_files");
    assert!(
        !files.is_empty(),
        "first (HEAD) commit should change at least one file"
    );
}

#[test]
fn git_cli_repo_changed_files_head_is_modified() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GitCliRepo::open(tiny.dir.path()).expect("open");
    // HEAD commit in tiny_repo is "expand greeting" — modifies src/main.rs only
    let files = repo.changed_files(&tiny.head_sha).expect("changed_files");
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, "src/main.rs");
    assert!(matches!(
        files[0].change_type,
        codelore_lib::ChangeType::Modified
    ));
}

#[test]
fn git_cli_repo_extracts_hunks_for_a_modified_file() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GitCliRepo::open(tiny.dir.path()).expect("open");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        ..Options::default()
    };
    let commits: Vec<_> = repo
        .walk_commits(&opts)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    // HEAD is a modify commit — should have exactly 1 hunk on src/main.rs
    let head = &commits[0];
    let hunks = repo
        .diff_hunks(&head.rev, "src/main.rs")
        .expect("diff_hunks");
    assert!(
        !hunks.is_empty(),
        "modified file should have at least 1 hunk"
    );
}

#[test]
fn git_cli_repo_extracts_hunks_for_root_commit() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GitCliRepo::open(tiny.dir.path()).expect("open");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        ..Options::default()
    };
    let commits: Vec<_> = repo
        .walk_commits(&opts)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    // Last commit (oldest) is the root commit — no parent, but git show -p handles it
    let root = commits.last().expect("at least one commit");
    assert!(
        root.parents.is_empty(),
        "root commit should have no parents"
    );
    let hunks = repo
        .diff_hunks(&root.rev, "src/main.rs")
        .expect("diff_hunks for root");
    assert!(
        !hunks.is_empty(),
        "root commit should have hunks for added file"
    );
}

#[test]
fn git_cli_repo_commit_metadata_unsigned() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GitCliRepo::open(tiny.dir.path()).expect("open");
    let meta = repo
        .commit_metadata(&tiny.head_sha)
        .expect("commit_metadata");
    assert_eq!(meta.rev, tiny.head_sha);
    assert!(!meta.signed, "tiny_repo commits are not GPG-signed");
    assert!(meta.signed_by.is_none());
    assert!(
        meta.signoffs.is_empty(),
        "no Signed-off-by trailers in tiny_repo"
    );
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn run_git(path: &std::path::Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(path)
        .args(args)
        .status()
        .expect("git");
    assert!(status.success(), "git {args:?} failed");
}
