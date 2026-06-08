//! `messages` analysis integration tests.

use codelore_lib::Options;
use codelore_lib::analyses::messages::run_messages;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;

fn run_git(path: &std::path::Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(path)
        .output()
        .expect("git");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn write(p: std::path::PathBuf, content: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

#[test]
fn messages_matches_regex_against_commit_messages() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    run_git(path, &["init", "-b", "main", "--quiet"]);
    run_git(path, &["config", "user.email", "t@e.com"]);
    run_git(path, &["config", "user.name", "T"]);

    // 5 commits over auth.rs with different message shapes.
    let scenarios: &[(&str, &str)] = &[
        ("bug fix #1", "v1"),
        ("feature X", "v2"),
        ("Bugfix typo", "v3"),
        ("WIP", "v4"),
        ("bug found", "v5"),
    ];
    for (i, (msg, body)) in scenarios.iter().enumerate() {
        write(path.join(format!("auth_{i}.rs")), body);
        run_git(path, &["add", "."]);
        run_git(path, &["commit", "-m", msg, "--quiet"]);
    }

    let repo = GixRepo::open(path).expect("gix open");
    let db = FactsDb::new_in_memory().expect("db");

    // Case-sensitive: "bug" matches "bug fix #1" and "bug found" only
    // ("Bugfix typo" has capital B).
    let opts = Options {
        repo_path: path.to_path_buf(),
        min_revs: 0,
        message_regex: Some("bug".into()),
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");
    let rows = run_messages(&db, &opts).expect("messages");
    let total: u32 = rows.iter().map(|r| r.matches).sum();
    assert_eq!(
        total, 2,
        "case-sensitive 'bug' → matches commits 1 + 5 (each touches 1 file) — got {rows:?}"
    );

    // Case-insensitive: "(?i)bug" matches all 3 bug-themed commits.
    let opts_i = Options {
        message_regex: Some("(?i)bug".into()),
        ..opts.clone()
    };
    let rows_i = run_messages(&db, &opts_i).expect("messages case-insensitive");
    let total_i: u32 = rows_i.iter().map(|r| r.matches).sum();
    assert_eq!(
        total_i, 3,
        "(?i)bug → matches commits 1, 3, 5 — got {rows_i:?}"
    );
}

#[test]
fn messages_returns_error_when_regex_not_provided() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    run_git(path, &["init", "-b", "main", "--quiet"]);
    run_git(path, &["config", "user.email", "t@e.com"]);
    run_git(path, &["config", "user.name", "T"]);
    write(path.join("x.rs"), "x");
    run_git(path, &["add", "."]);
    run_git(path, &["commit", "-m", "init", "--quiet"]);

    let repo = GixRepo::open(path).expect("gix open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: path.to_path_buf(),
        min_revs: 0,
        message_regex: None,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let err = run_messages(&db, &opts).expect_err("must error without regex");
    let msg = format!("{err}");
    assert!(
        msg.contains("--expression-to-match"),
        "error should mention the missing flag: {msg}"
    );
}

#[test]
fn messages_rejects_invalid_regex() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    run_git(path, &["init", "-b", "main", "--quiet"]);
    run_git(path, &["config", "user.email", "t@e.com"]);
    run_git(path, &["config", "user.name", "T"]);
    write(path.join("x.rs"), "x");
    run_git(path, &["add", "."]);
    run_git(path, &["commit", "-m", "init", "--quiet"]);

    let repo = GixRepo::open(path).expect("gix open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: path.to_path_buf(),
        min_revs: 0,
        // Unclosed group — invalid Rust regex.
        message_regex: Some("(unclosed".into()),
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let err = run_messages(&db, &opts).expect_err("must error on invalid regex");
    let msg = format!("{err}");
    assert!(
        msg.contains("invalid --expression-to-match"),
        "error should mention the bad regex: {msg}"
    );
}
