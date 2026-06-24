//! End-to-end coverage of `run_stale_code` against an ingested `FactsDb`.
//!
//! Stale-code uses two fixed thresholds today (≥12 months untouched,
//! cognitive ≤ 5). The `tiny_repo` fixture's commits are all
//! present-day, so stale-code legitimately returns an empty result.
//! The point of this test is to exercise the SQL end-to-end against a
//! real `DuckDB` — a typo or schema rename (the exact regression class
//! that bit stale-code's `DATE_DIFF` binder error in the validation
//! pass) would surface here at CI time instead of at customer runtime.

use codelore_lib::Options;
use codelore_lib::analyses::stale_code::run_stale_code;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;

#[test]
fn stale_code_executes_cleanly_on_tiny_repo() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    // `tiny_repo` has only fresh commits → stale-code returns empty.
    // The SQL must still execute cleanly; an empty `Vec` is the
    // success signal, not a `Result::Err`.
    let rows = run_stale_code(&db, &opts).expect("run stale-code");
    assert!(
        rows.is_empty(),
        "tiny_repo has only fresh commits — expected empty stale-code result, got {} rows",
        rows.len(),
    );
}

/// Build a repo with one long-untouched file (`legacy.txt`, last
/// committed early-2024) and one recent file (`recent.txt`, committed
/// 2026). With the staleness anchor pinned to the newest commit, the
/// legacy file is ~24 months stale and must surface.
fn build_aged_repo() -> tempfile::TempDir {
    use std::process::Command;
    fn run(path: &std::path::Path, date: &str, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(path)
            .args(args)
            .env("GIT_AUTHOR_DATE", date)
            .env("GIT_COMMITTER_DATE", date)
            .status()
            .expect("git");
        assert!(status.success());
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path();
    run(
        path,
        "2024-01-01T00:00:00Z",
        &["init", "-b", "main", "--quiet"],
    );
    run(
        path,
        "2024-01-01T00:00:00Z",
        &["config", "user.email", "t@t"],
    );
    run(
        path,
        "2024-01-01T00:00:00Z",
        &["config", "user.name", "Tiny"],
    );

    std::fs::write(path.join("legacy.txt"), "old\n").unwrap();
    run(path, "2022-01-15T12:00:00Z", &["add", "legacy.txt"]);
    run(
        path,
        "2022-01-15T12:00:00Z",
        &["commit", "-m", "legacy", "--quiet"],
    );

    // `recent.txt` is the newest commit. It is old enough (years ago)
    // that a WALL-CLOCK anchor would flag it as stale, but because it
    // IS the newest commit it must read 0 months stale under the
    // deterministic max-commit-date anchor — making it the test's
    // discriminator between the two anchor strategies.
    std::fs::write(path.join("recent.txt"), "new\n").unwrap();
    run(path, "2024-01-15T12:00:00Z", &["add", "recent.txt"]);
    run(
        path,
        "2024-01-15T12:00:00Z",
        &["commit", "-m", "recent", "--quiet"],
    );
    dir
}

#[test]
fn stale_code_anchor_defaults_to_newest_commit_deterministically() {
    let aged = build_aged_repo();
    let repo = GixRepo::open(aged.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    // No `age_time_now` set — the staleness anchor must default to the
    // newest commit date in the store, NOT the wall clock. That makes
    // the result deterministic across runs on the same cached store.
    let opts = Options {
        repo_path: aged.path().to_path_buf(),
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let first = run_stale_code(&db, &opts).expect("run stale-code");
    let second = run_stale_code(&db, &opts).expect("run stale-code again");

    // Determinism: two runs against the same store produce identical
    // output (the wall-clock anchor would drift second-to-second).
    let project = |rows: &[codelore_lib::analyses::stale_code::StaleCodeRow]| {
        rows.iter()
            .map(|r| {
                (
                    r.path.clone(),
                    r.last_touched.clone(),
                    r.months_since_touched,
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        project(&first),
        project(&second),
        "stale-code output must be deterministic across runs"
    );

    // The legacy file is ~24 months older than the newest commit and
    // must surface; the recent file (anchor itself) is 0 months stale.
    let legacy = first
        .iter()
        .find(|r| r.path == "legacy.txt")
        .expect("legacy.txt must surface as stale");
    assert!(
        legacy.months_since_touched >= 12,
        "legacy.txt should be >=12 months stale relative to the newest commit; got {}",
        legacy.months_since_touched,
    );
    assert!(
        !first.iter().any(|r| r.path == "recent.txt"),
        "recent.txt (the anchor commit) is not stale and must not surface",
    );
}
