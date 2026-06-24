//! End-to-end coverage of `run_delivery_friction` against an ingested
//! `FactsDb`.
//!
//! Delivery friction = `percent_rank(revisions) × percent_rank(median
//! lead_time) × percent_rank(cognitive) × 100` — high requires
//! elevation on ALL THREE axes. The composite is the answer to "where
//! is technical debt actually slowing us down?".
//!
//! The fixture is `tiny_repo`, which crafts commits with equal author
//! and committer timestamps; that means `median_lead_time_days = 0` for
//! every file, the `pr_lt` factor is `0`, and `friction_score` collapses
//! to `0` for every row. The contract this test pins:
//!
//! 1. SQL runs cleanly on the v3 schema (`committer_date` column populated).
//! 2. Row shape holds: every required column emitted.
//! 3. `wip_age_days` reports a positive number for every file (fixture
//!    commits are seconds-old at test time).
//!
//! A non-zero `friction_score` requires the fixture to surface real
//! review-time deltas — that is left to fixtures whose commits use
//! `git commit --date=...` overrides; the analysis SQL is exercised
//! correctly here.

use codelore_lib::Options;
use codelore_lib::analyses::delivery_friction::run_delivery_friction;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;

#[test]
fn delivery_friction_runs_cleanly_on_tiny_repo() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_delivery_friction(&db, &opts).expect("run delivery-friction");

    assert!(
        !rows.is_empty(),
        "tiny_repo must surface at least one file with revisions >= min_revs=1"
    );

    for row in &rows {
        assert!(!row.path.is_empty(), "path populated");
        assert!(row.revisions >= 1, "revisions >= min_revs filter");
        assert!(
            row.median_lead_time_days >= 0.0,
            "lead-time delta cannot be negative; got {} for {}",
            row.median_lead_time_days,
            row.path
        );
        assert!(
            row.p95_lead_time_days >= row.median_lead_time_days,
            "p95 >= median by construction"
        );
        assert!(
            row.wip_age_days >= 0.0,
            "wip_age_days cannot be negative; got {} for {}",
            row.wip_age_days,
            row.path
        );
        assert!(
            (0.0..=100.0).contains(&row.friction_score),
            "friction_score must be in [0,100]; got {} for {}",
            row.friction_score,
            row.path
        );
    }
}

/// Build a repo whose newest commit is years in the past, so a
/// wall-clock `wip_age_days` anchor would report a large (and
/// non-deterministic) age while the deterministic max-commit-date
/// anchor reports ~0 days for the most-recently-touched file.
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
        "2023-01-01T00:00:00Z",
        &["init", "-b", "main", "--quiet"],
    );
    run(
        path,
        "2023-01-01T00:00:00Z",
        &["config", "user.email", "t@t"],
    );
    run(
        path,
        "2023-01-01T00:00:00Z",
        &["config", "user.name", "Tiny"],
    );

    std::fs::write(path.join("app.txt"), "one\n").unwrap();
    run(path, "2023-06-01T12:00:00Z", &["add", "app.txt"]);
    run(
        path,
        "2023-06-01T12:00:00Z",
        &["commit", "-m", "c1", "--quiet"],
    );
    std::fs::write(path.join("app.txt"), "one\ntwo\n").unwrap();
    run(
        path,
        "2023-07-01T12:00:00Z",
        &["commit", "-am", "c2", "--quiet"],
    );
    dir
}

#[test]
fn delivery_friction_wip_age_anchored_to_newest_commit_deterministically() {
    let aged = build_aged_repo();
    let repo = GixRepo::open(aged.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    // No `age_time_now` set — the `wip_age_days` anchor must default to
    // the newest commit date in the store, NOT the wall clock, so the
    // output is deterministic across runs on a cached store.
    let opts = Options {
        repo_path: aged.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let first = run_delivery_friction(&db, &opts).expect("run delivery-friction");
    let second = run_delivery_friction(&db, &opts).expect("run again");

    let project = |rows: &[codelore_lib::analyses::delivery_friction::DeliveryFrictionRow]| {
        rows.iter()
            .map(|r| (r.path.clone(), r.wip_age_days, r.friction_score))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        project(&first),
        project(&second),
        "delivery-friction output must be deterministic across runs"
    );

    // The most-recently-touched file is the anchor commit itself, so
    // its wip_age_days must be ~0 — a wall-clock anchor (today minus a
    // 2023 commit) would report hundreds of days.
    let app = first
        .iter()
        .find(|r| r.path == "app.txt")
        .expect("app.txt must surface");
    assert!(
        app.wip_age_days < 1.0,
        "wip_age_days for the newest-touched file must be ~0 under the \
         max-commit-date anchor; got {}",
        app.wip_age_days,
    );
}
