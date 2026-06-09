use codelore_lib::repo::{GixRepo, Repo};

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
    let canonical = repo.resolve_alias("", "alice@old.com");
    assert_eq!(canonical, "alice@real.com", "mailmap should map old → real");

    let unknown = repo.resolve_alias("", "bob@example.com");
    assert_eq!(
        unknown, "bob@example.com",
        "unknown email passes through unchanged"
    );
}

/// `.mailmap` supports a 4-token rule that matches on Name+Email together:
///   `Canonical Name <canonical@email> Old Name <old@email>`
/// Earlier versions of `resolve_alias` passed only the email to gix's
/// mailmap, so these rules never matched — `GitCliRepo::walk_commits` then
/// silently disagreed with `GixRepo::walk_commits` (which had its own
/// inline name+email resolution) on any `.mailmap` using this form. This
/// test guards that the trait method now honours the name too.
#[test]
fn mailmap_name_plus_email_rule_resolves() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path();
    run_git(path, &["init", "-b", "main", "--quiet"]);
    run_git(path, &["config", "user.email", "carol@old.com"]);
    run_git(path, &["config", "user.name", "Carol Old"]);

    // 4-token rule: matches ONLY when both name AND email match.
    std::fs::write(
        path.join(".mailmap"),
        "Carol Real <carol@real.com> Carol Old <carol@old.com>\n",
    )
    .unwrap();

    std::fs::write(path.join("README.md"), "hello\n").unwrap();
    run_git(path, &["add", "."]);
    run_git(path, &["commit", "-m", "init", "--quiet"]);

    let repo = GixRepo::open(path).expect("open");
    // With the correct name, the rule matches and we get the canonical email.
    let with_name = repo.resolve_alias("Carol Old", "carol@old.com");
    assert_eq!(
        with_name, "carol@real.com",
        "name+email rule should match when both are passed"
    );
    // With an empty name, the rule should NOT match (4-token rules require
    // exact name match — gix does not auto-fallback to email-only matching).
    let without_name = repo.resolve_alias("", "carol@old.com");
    assert_eq!(
        without_name, "carol@old.com",
        "name+email rule must NOT match on email alone"
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
