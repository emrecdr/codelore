use codelore_lib::Options;
use codelore_lib::analyses::code_age::run_code_age;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;

#[test]
fn code_age_for_tiny_repo() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_code_age(&db, &opts).expect("run");
    assert!(!rows.is_empty(), "should produce at least 1 row");

    // All ages must be >= 0
    for row in &rows {
        assert!(
            row.age_months >= 0,
            "age must be >= 0, got {} for {}",
            row.age_months,
            row.path
        );
    }

    // tiny_repo commits happen "now" → age in months should be 0
    let main_row = rows
        .iter()
        .find(|r| r.path == "src/main.rs")
        .expect("main.rs");
    assert!(
        main_row.age_months <= 1,
        "tiny_repo just built; main.rs age should be ≤1 month, got {}",
        main_row.age_months
    );
}

/// PAR-2 regression: `--age-time-now <past-date>` must drop commits that
/// occurred AFTER the anchor. Before this fix, `code-age` returned
/// negative `age_months` values for files whose latest commit was past
/// the anchor; code-maat's `changes-within-time-span` correctly drops
/// them. With the WHERE filter in place, files whose only commits are
/// after the anchor disappear from the output entirely.
#[test]
fn code_age_back_test_excludes_post_anchor_commits() {
    use time::macros::date;
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        // tiny_repo commits land at "now" (2026-06-09+); a back-test
        // anchored before any commit must drop everything.
        age_time_now: Some(date!(2020 - 01 - 01)),
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_code_age(&db, &opts).expect("run");
    assert!(
        rows.is_empty(),
        "all commits post-anchor → no rows; got {} rows",
        rows.len()
    );
}

/// PAR-4 regression: `age_months` uses interval-month (whole calendar
/// months elapsed) semantics, NOT month-boundary-crossing. Three table
/// cases drawn from the doc-comment in `code_age.rs`:
///
///   anchor 2026-04-01, last 2026-03-15  → interval 0 (boundary-cross 1)
///   anchor 2026-04-16, last 2026-03-15  → interval 1 (boundary-cross 1)
///   anchor 2026-04-30, last 2026-03-31  → interval 0 (boundary-cross 1)
///
/// We build a one-commit repo at each `last` date, then run `code-age`
/// with the matching anchor and assert the expected interval value.
#[test]
fn code_age_uses_interval_month_semantics_not_boundary_crossing() {
    use std::process::Command;
    use time::macros::date;

    fn build_repo_with_commit_at(date_str: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path();
        let init = Command::new("git")
            .arg("-C")
            .arg(path)
            .args(["init", "-b", "main", "--quiet"])
            .status()
            .expect("git init");
        assert!(init.success());
        for (k, v) in [("user.email", "tiny@example.com"), ("user.name", "Tiny")] {
            Command::new("git")
                .arg("-C")
                .arg(path)
                .args(["config", k, v])
                .status()
                .expect("git config");
        }
        std::fs::write(path.join("a.txt"), "x\n").expect("write");
        Command::new("git")
            .arg("-C")
            .arg(path)
            .args(["add", "a.txt"])
            .status()
            .expect("git add");
        let status = Command::new("git")
            .arg("-C")
            .arg(path)
            .args(["commit", "-m", "init", "--quiet"])
            .env("GIT_AUTHOR_DATE", date_str)
            .env("GIT_COMMITTER_DATE", date_str)
            .status()
            .expect("git commit");
        assert!(status.success());
        dir
    }

    let cases: &[(&str, time::Date, i32)] = &[
        // (commit ISO date, anchor date, expected age_months)
        ("2026-03-15T12:00:00Z", date!(2026 - 04 - 01), 0),
        ("2026-03-15T12:00:00Z", date!(2026 - 04 - 16), 1),
        ("2026-03-31T12:00:00Z", date!(2026 - 04 - 30), 0),
        // Sanity: same day → 0 months.
        ("2026-03-15T12:00:00Z", date!(2026 - 03 - 15), 0),
        // Sanity: exactly one calendar year → 12 months.
        ("2025-04-15T12:00:00Z", date!(2026 - 04 - 15), 12),
    ];

    for (commit_iso, anchor, expected) in cases {
        let repo_dir = build_repo_with_commit_at(commit_iso);
        let repo = GixRepo::open(repo_dir.path()).expect("open");
        let db = FactsDb::new_in_memory().expect("db");
        let opts = Options {
            repo_path: repo_dir.path().to_path_buf(),
            min_revs: 1,
            age_time_now: Some(*anchor),
            ..Options::default()
        };
        db.ingest(&repo, &opts).expect("ingest");
        let rows = run_code_age(&db, &opts).expect("run");
        let row = rows
            .iter()
            .find(|r| r.path == "a.txt")
            .unwrap_or_else(|| panic!("missing a.txt row for case {commit_iso} → {anchor}"));
        assert_eq!(
            row.age_months, *expected,
            "interval-month semantics for case {commit_iso} → {anchor}: expected {expected}, got {}",
            row.age_months,
        );
    }
}

/// PAR-2 regression: when the anchor is RECENT and commits straddle
/// the anchor, files with only post-anchor commits drop while files
/// with at least one pre-anchor commit retain the pre-anchor max as
/// their reference and never produce a negative `age_months`.
#[test]
fn code_age_never_returns_negative_age() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    // Anchor = today (default). Without the PAR-2 filter this would still
    // be fine; but anchor a few days in the past relative to test runs
    // and the negative-age bug would re-surface without the filter.
    let anchor = (time::OffsetDateTime::now_utc() - time::Duration::days(30)).date();
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        age_time_now: Some(anchor),
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");
    let rows = run_code_age(&db, &opts).expect("run");
    for row in &rows {
        assert!(
            row.age_months >= 0 && row.age_days >= 0,
            "negative age leaked through filter for {}: months={} days={}",
            row.path,
            row.age_months,
            row.age_days
        );
    }
}

/// A file added and then DELETED before the analysis
/// runs must NOT appear in `code-age` output. Previously deleted files
/// were included (cluttered output with historical noise). The
/// `live_paths_at_anchor` CTE in `code_age.rs`'s SQL drops them.
///
/// The fixture: build a tiny git history where one file is added then
/// deleted; assert it's missing from the run.
#[test]
#[allow(clippy::items_after_statements)]
fn code_age_excludes_deleted_files() {
    use std::process::Command;
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path();
    fn run_git_at(path: &std::path::Path, date: &str, args: &[&str]) {
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
    run_git_at(
        path,
        "2026-01-01T00:00:00Z",
        &["init", "-b", "main", "--quiet"],
    );
    run_git_at(
        path,
        "2026-01-01T00:00:00Z",
        &["config", "user.email", "t@t"],
    );
    run_git_at(
        path,
        "2026-01-01T00:00:00Z",
        &["config", "user.name", "Tiny"],
    );

    // alive.rs and gone.rs both added on day 1
    std::fs::write(path.join("alive.rs"), "x").unwrap();
    std::fs::write(path.join("gone.rs"), "y").unwrap();
    run_git_at(
        path,
        "2026-01-01T12:00:00Z",
        &["add", "alive.rs", "gone.rs"],
    );
    run_git_at(
        path,
        "2026-01-01T12:00:00Z",
        &["commit", "-m", "add both", "--quiet"],
    );

    // gone.rs deleted on day 2
    std::fs::remove_file(path.join("gone.rs")).unwrap();
    run_git_at(path, "2026-01-02T12:00:00Z", &["add", "-A"]);
    run_git_at(
        path,
        "2026-01-02T12:00:00Z",
        &["commit", "-m", "del", "--quiet"],
    );

    let repo = GixRepo::open(path).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: path.to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_code_age(&db, &opts).expect("run");
    let paths: Vec<&str> = rows.iter().map(|r| r.path.as_str()).collect();
    assert!(
        paths.contains(&"alive.rs"),
        "alive.rs must surface; got paths={paths:?}",
    );
    assert!(
        !paths.contains(&"gone.rs"),
        "gone.rs was deleted — must NOT surface; got paths={paths:?}",
    );
}
