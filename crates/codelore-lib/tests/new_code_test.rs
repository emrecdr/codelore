//! Integration coverage for `run_new_code_scope` (the `[new_code]` gate's
//! impure born/touched partition).
//!
//! Only the pure `born_touched_flags` helper had coverage before this file —
//! the SQL/window path that resolves the window-start rev, joins it against
//! live-at-HEAD code-health rows, and (for the touched band) parses the
//! window-start blob via the `Repo` trait was untested end to end.

use codelore_lib::Options;
use codelore_lib::analyses::code_health::run_code_health;
use codelore_lib::analyses::new_code::run_new_code_scope;
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

fn commit_at(path: &std::path::Path, msg: &str, date: &str) {
    run_git(path, &["add", "."]);
    let out = std::process::Command::new("git")
        .args(["commit", "-m", msg, "--quiet"])
        .env("GIT_AUTHOR_DATE", date)
        .env("GIT_COMMITTER_DATE", date)
        .current_dir(path)
        .output()
        .expect("commit");
    assert!(
        out.status.success(),
        "git commit failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn init_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path();
    run_git(path, &["init", "-b", "main", "--quiet"]);
    run_git(path, &["config", "user.email", "t@e.com"]);
    run_git(path, &["config", "user.name", "T"]);
    dir
}

/// Three commits straddling a 90-day window boundary (window anchors to
/// `MAX(commits.date)`, here 2020-06-01, so the boundary sits ~2020-03-03):
///
/// - `legacy.rs` is added at the baseline commit (2020-01-01, well before the
///   boundary) and modified again inside the window (2020-05-01) — touched,
///   not born.
/// - `fresh.rs` is added inside the window (2020-06-01, the HEAD commit) —
///   born (born ⊂ touched).
///
/// The baseline commit is the pre-window rev `window_start_rev` resolves to.
fn build_straddling_repo() -> tempfile::TempDir {
    let dir = init_repo();
    let path = dir.path();

    std::fs::write(
        path.join("legacy.rs"),
        "pub fn legacy() -> i32 {\n    1\n}\n",
    )
    .expect("write legacy.rs baseline");
    commit_at(path, "add legacy", "2020-01-01T00:00:00");

    std::fs::write(
        path.join("legacy.rs"),
        "pub fn legacy() -> i32 {\n    2\n}\n",
    )
    .expect("write legacy.rs evolved");
    commit_at(path, "evolve legacy", "2020-05-01T00:00:00");

    std::fs::write(path.join("fresh.rs"), "pub fn fresh() -> i32 {\n    3\n}\n")
        .expect("write fresh.rs");
    commit_at(path, "add fresh", "2020-06-01T00:00:00");

    dir
}

/// Two commits, both inside the trailing 90-day window (boundary
/// ~2020-03-03) — no commit predates the window, so `window_start_rev`
/// resolves to `None` and the whole repository is the shallow-history skip.
fn build_no_baseline_repo() -> tempfile::TempDir {
    let dir = init_repo();
    let path = dir.path();

    std::fs::write(path.join("a.rs"), "pub fn a() -> i32 {\n    1\n}\n").expect("write a.rs");
    commit_at(path, "add a", "2020-05-15T00:00:00");

    std::fs::write(path.join("b.rs"), "pub fn b() -> i32 {\n    2\n}\n").expect("write b.rs");
    commit_at(path, "add b", "2020-06-01T00:00:00");

    dir
}

#[test]
fn run_new_code_scope_partitions_born_vs_touched_with_baseline() {
    let fixture = build_straddling_repo();
    let repo = GixRepo::open(fixture.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: fixture.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let health = run_code_health(&db, &opts).expect("code health");
    let scope = run_new_code_scope(&db, &repo, &opts, 90, &health).expect("new code scope");

    assert!(
        scope.window_start_present,
        "the 2020-01-01 baseline commit predates the 90-day window"
    );

    let born_paths: Vec<&str> = scope.born.iter().map(|(p, _)| p.as_str()).collect();
    assert_eq!(
        born_paths,
        vec!["fresh.rs"],
        "fresh.rs's only commit lands inside the window, so it is born"
    );

    let touched_paths: Vec<&str> = scope.touched.iter().map(|(p, _)| p.as_str()).collect();
    assert_eq!(
        touched_paths,
        vec!["legacy.rs"],
        "legacy.rs predates the window but was modified inside it, so it is touched, not born"
    );
}

#[test]
fn run_new_code_scope_skips_when_no_pre_window_baseline() {
    let fixture = build_no_baseline_repo();
    let repo = GixRepo::open(fixture.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: fixture.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let health = run_code_health(&db, &opts).expect("code health");
    let scope = run_new_code_scope(&db, &repo, &opts, 90, &health).expect("new code scope");

    assert!(
        !scope.window_start_present,
        "every commit sits inside the window, so there is no pre-window baseline"
    );
    assert!(
        scope.born.is_empty(),
        "the shallow-history skip returns an empty born band"
    );
    assert!(
        scope.touched.is_empty(),
        "the shallow-history skip returns an empty touched band"
    );
}
