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

/// Build a repo where `zombie.txt` is added, then `git rm`-ed, then
/// re-added. Its latest change is the re-add, so it is LIVE at HEAD
/// despite the intervening deletion. A "never deleted" liveness filter
/// wrongly excludes it; a live-at-HEAD filter keeps it. `recent.txt`
/// pins the staleness anchor to 2024 so the re-added file reads as
/// long-untouched.
fn build_deleted_then_readded_repo() -> tempfile::TempDir {
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
        "2022-01-15T12:00:00Z",
        &["init", "-b", "main", "--quiet"],
    );
    run(
        path,
        "2022-01-15T12:00:00Z",
        &["config", "user.email", "t@t"],
    );
    run(
        path,
        "2022-01-15T12:00:00Z",
        &["config", "user.name", "Tiny"],
    );

    std::fs::write(path.join("zombie.txt"), "first life\n").unwrap();
    run(path, "2022-01-15T12:00:00Z", &["add", "zombie.txt"]);
    run(
        path,
        "2022-01-15T12:00:00Z",
        &["commit", "-m", "add zombie", "--quiet"],
    );

    run(
        path,
        "2022-02-15T12:00:00Z",
        &["rm", "--quiet", "zombie.txt"],
    );
    run(
        path,
        "2022-02-15T12:00:00Z",
        &["commit", "-m", "remove zombie", "--quiet"],
    );

    std::fs::write(path.join("zombie.txt"), "second life\n").unwrap();
    run(path, "2022-03-15T12:00:00Z", &["add", "zombie.txt"]);
    run(
        path,
        "2022-03-15T12:00:00Z",
        &["commit", "-m", "re-add zombie", "--quiet"],
    );

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
fn stale_code_surfaces_deleted_then_readded_file() {
    let repo_dir = build_deleted_then_readded_repo();
    let repo = GixRepo::open(repo_dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: repo_dir.path().to_path_buf(),
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    // The live-at-HEAD liveness fix is always on: `zombie.txt`'s latest
    // change is the re-add, so it surfaces as stale even though it was
    // deleted in between. Its `last_touched` is the re-add date, not the
    // intervening deletion.
    let rows = run_stale_code(&db, &opts).expect("run stale-code");
    let zombie = rows
        .iter()
        .find(|r| r.path == "zombie.txt")
        .expect("zombie.txt (deleted then re-added, live at HEAD) must surface as stale");
    assert!(
        zombie.months_since_touched >= 12,
        "zombie.txt should read >=12 months stale relative to the 2024 anchor; got {}",
        zombie.months_since_touched,
    );
    assert_eq!(
        zombie.last_touched, "2022-03-15",
        "last_touched must be the re-add date, not the intervening deletion",
    );
}

/// Build a repo where `legacy_old.txt` (a multi-line body, so gix's
/// default 50%-similarity rename detection registers a rename rather
/// than a delete + add) is `git mv`-ed to `legacy_new.txt`. A rename
/// records no `deleted` row for the old path, so under raw `changes` the
/// dead `legacy_old.txt` ghosts as a live, long-untouched file.
/// `recent.txt` pins the anchor to 2024.
fn build_renamed_repo() -> tempfile::TempDir {
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
        "2022-01-15T12:00:00Z",
        &["init", "-b", "main", "--quiet"],
    );
    run(
        path,
        "2022-01-15T12:00:00Z",
        &["config", "user.email", "t@t"],
    );
    run(
        path,
        "2022-01-15T12:00:00Z",
        &["config", "user.name", "Tiny"],
    );

    std::fs::write(path.join("legacy_old.txt"), "alpha\nbeta\ngamma\ndelta\n").unwrap();
    run(path, "2022-01-15T12:00:00Z", &["add", "legacy_old.txt"]);
    run(
        path,
        "2022-01-15T12:00:00Z",
        &["commit", "-m", "add legacy", "--quiet"],
    );

    run(
        path,
        "2022-02-15T12:00:00Z",
        &["mv", "legacy_old.txt", "legacy_new.txt"],
    );
    run(
        path,
        "2022-02-15T12:00:00Z",
        &["commit", "-m", "rename legacy", "--quiet"],
    );

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
fn stale_code_no_ghost_for_renamed_file_under_lineage() {
    let repo_dir = build_renamed_repo();
    let repo = GixRepo::open(repo_dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");

    // Ingest once: rename detection is on by default, so the `git mv`
    // lands as a single `renamed` row (no deletion under the old path).
    let opts_off = Options {
        repo_path: repo_dir.path().to_path_buf(),
        use_canonical_lineage: false,
        ..Options::default()
    };
    db.ingest(&repo, &opts_off).expect("ingest");

    // Lineage OFF: the dead `legacy_old.txt` ghosts as stale because the
    // rename left no deletion row under its old path. This documents the
    // ghost AND guards fixture integrity — a delete + add ingest would
    // drop the old path here, so its presence proves the rename was
    // detected.
    let rows_off = run_stale_code(&db, &opts_off).expect("run stale-code (lineage off)");
    assert!(
        rows_off.iter().any(|r| r.path == "legacy_old.txt"),
        "fixture integrity: legacy_old.txt must ghost as stale under raw changes \
         (proves git mv ingested as a rename); got: {rows_off:?}"
    );

    // Lineage ON: `changes_lineage` folds legacy_old.txt into
    // legacy_new.txt, so the ghost disappears and only the live canonical
    // path can surface.
    let opts_on = Options {
        repo_path: repo_dir.path().to_path_buf(),
        use_canonical_lineage: true,
        ..Options::default()
    };
    let rows_on = run_stale_code(&db, &opts_on).expect("run stale-code (lineage on)");
    assert!(
        !rows_on.iter().any(|r| r.path == "legacy_old.txt"),
        "legacy_old.txt must NOT ghost when canonical lineage is on; got: {rows_on:?}"
    );
    assert!(
        rows_on.iter().any(|r| r.path == "legacy_new.txt"),
        "the live canonical path legacy_new.txt should surface as stale; got: {rows_on:?}"
    );
}
