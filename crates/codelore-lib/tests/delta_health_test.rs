//! Integration test for the delta-health function-metric extraction:
//! function/method rows come back; class- and file-level complexity rows
//! do not leak in as functions.

use std::path::Path;
use std::process::Command;

use codelore_lib::cli_api::Options;
use codelore_lib::cli_api::analyses::delta_health::run_function_metrics;
use codelore_lib::cli_api::facts::FactsDb;
use codelore_lib::cli_api::repo::GixRepo;

fn git(repo: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .output()
        .expect("spawn git");
    assert!(out.status.success(), "git {args:?}: {out:?}");
}

#[test]
fn extracts_function_rows_only() {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = dir.path();
    git(repo, &["init", "-q"]);
    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::write(
        repo.join("src/lib.rs"),
        "pub fn tiny() -> i32 {\n    1\n}\n\npub fn also_tiny() -> i32 {\n    2\n}\n",
    )
    .unwrap();
    git(repo, &["add", "."]);
    git(repo, &["commit", "-q", "-m", "init"]);

    let opts = Options {
        repo_path: repo.to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    let gix = GixRepo::open(repo).expect("open repo");
    let db = FactsDb::new_in_memory().expect("in-memory facts");
    db.ingest(&gix, &opts).expect("ingest");

    let rows = run_function_metrics(&db).expect("extract");
    let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
    // Entity names are stored as "{fn_name}@{start}-{end}"; check prefix.
    assert!(
        names.iter().any(|n| n.starts_with("tiny")),
        "missing fn tiny in {names:?}"
    );
    assert!(
        names.iter().any(|n| n.starts_with("also_tiny")),
        "missing fn also_tiny in {names:?}"
    );
    // No file-level unit row masquerading as a function.
    assert!(
        rows.iter()
            .all(|r| r.name != "src/lib.rs" && !r.name.is_empty()),
        "file/unit rows leaked: {names:?}"
    );
    for r in &rows {
        assert_eq!(r.path, "src/lib.rs");
        assert!(r.loc >= 1, "loc should be populated, got {}", r.loc);
    }
}
