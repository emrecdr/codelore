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
    // Names are bare (the `@{start}-{end}` span suffix is stripped so the
    // pairing key is stable across revisions).
    assert!(names.contains(&"tiny"), "missing fn tiny in {names:?}");
    assert!(
        names.contains(&"also_tiny"),
        "missing fn also_tiny in {names:?}"
    );
    assert!(
        rows.iter().all(|r| !r.name.contains('@')),
        "names must be bare, no line-span suffix: {names:?}"
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

/// Same-named methods on different types in one file share a bare name once
/// the line-span suffix is stripped; they must collapse to a single
/// worst-case row so base↔head pairing has a stable, unique key.
#[test]
fn overloaded_names_collapse_to_worst_case() {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = dir.path();
    git(repo, &["init", "-q"]);
    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::write(
        repo.join("src/lib.rs"),
        "pub struct A;\npub struct B;\nimpl A {\n    pub fn build(&self) -> i32 {\n        1\n    }\n}\nimpl B {\n    pub fn build(&self, x: i32) -> i32 {\n        let mut acc = 0;\n        for i in 0..x {\n            if i % 2 == 0 {\n                acc += i;\n            } else {\n                acc -= i;\n            }\n        }\n        acc\n    }\n}\n",
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
    let build_rows: Vec<_> = rows.iter().filter(|r| r.name == "build").collect();
    // codelore-rca names a method space by its bare identifier, so both
    // `A::build` and `B::build` are stored as `build@<span>` and genuinely
    // collide once the span is stripped. They must aggregate to exactly one
    // row — asserting `== 1` (not `<= 1`) gives positive evidence the
    // GROUP BY MAX path fired, rather than passing vacuously on an empty
    // result.
    assert_eq!(
        build_rows.len(),
        1,
        "overloaded `build` methods must aggregate to exactly one row: {:?}",
        rows.iter().map(|r| (&r.name, r.loc)).collect::<Vec<_>>()
    );
    // The aggregated row carries the worst-case metrics: `B::build` (with the
    // loop body) is larger than the one-line `A::build`, so MAX(loc) must pick
    // it. A value this high is only reachable by the larger method — proof the
    // aggregation kept the worst case, not an arbitrary member.
    assert!(
        build_rows[0].loc >= 5,
        "aggregated build must hold worst-case loc, got {}",
        build_rows[0].loc
    );
}
