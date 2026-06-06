use bca_lib::repo::{GixRepo, Repo};

#[test]
fn mailmap_maps_email_to_canonical() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path();

    // Init repo + write .mailmap + commit
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

    let repo = GixRepo::open(path).expect("open");
    let canonical = repo.resolve_alias("alice@old.com");
    assert_eq!(canonical, "alice@real.com", "mailmap should map old → real");

    let unknown = repo.resolve_alias("bob@example.com");
    assert_eq!(
        unknown, "bob@example.com",
        "unknown email passes through unchanged"
    );
}

fn run_git(path: &std::path::Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(path)
        .args(args)
        .status()
        .expect("git");
    assert!(status.success(), "git {args:?} failed");
}
