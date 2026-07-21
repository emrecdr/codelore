//! End-to-end coverage of `run_delivery_metrics` against the `delivery_repo`
//! fixture.
//!
//! # Fixture summary (relevant for hand-computing expected values)
//!
//! Two `--no-ff` merges:
//!
//! - Branch 1 (feature/branch1): 3 branch-side commits spanning Jan 16
//!   10:00 → Jan 17 14:00; merged Jan 18 10:00.
//!   `branch_duration` = 18 Jan 10:00 − 16 Jan 10:00 = 48 h.
//!   Files on branch: `src/branch1.rs` only → `batch_size_files` = 1.
//! - Branch 2 (feature/branch2): 1 branch-side commit on Mar 7 10:00;
//!   merged Mar 7 14:00.
//!   `branch_duration` = 4 h.
//!   Files on branch: `src/branch2.rs` only → `batch_size_files` = 1.
//!
//! `PERCENTILE_CONT(0.50)` on `[48, 4]` = (48 + 4) / 2 = 26.0 h.
//!
//! `PERCENTILE_CONT(0.50)` on `[1, 1]` = 1.0 (`batch_size_files`).
//!
//! Two commits with a positive author→committer gap (non-merge):
//!
//! - Day 8: Bob `rework.rs`, author 2026-01-08 10:00, committer 2026-01-09 10:00 → 24 h
//! - Day 62: Bob `stable.rs`, author 2026-03-03 10:00, committer 2026-03-04 10:00 → 24 h
//!
//! `PERCENTILE_CONT(0.50)` on `[24, 24]` = 24.0 h.
//!
//! Rework signal: Day 5 Alice expands `rework.rs` (+2 lines); Day 8 Bob
//! trims it back (−2 lines) within a 3-day pair gap. With the default 21-day
//! window anchored to HEAD (`2026-04-21`), this event is ~110 days before HEAD
//! and is correctly excluded. The `rework_pct_is_positive` test uses
//! `rework_window_days = 365` to bring the signal into scope.
//!
//! All tests use `include_merges: true`; without it the `commit_parents` table
//! holds no merge rows and branch metrics would be empty.

use codelore_lib::Options;
use codelore_lib::analyses::delivery_metrics::run_delivery_metrics;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;

fn base_opts(path: &std::path::Path) -> Options {
    Options {
        repo_path: path.to_path_buf(),
        include_merges: true,
        min_revs: 1,
        ..Options::default()
    }
}

#[test]
fn branch_duration_p50_is_26_hours() {
    let fixture = codelore_lib::test_support::delivery_repo::build();
    let repo = GixRepo::open(fixture.dir.path()).expect("open repo");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = base_opts(fixture.dir.path());
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_delivery_metrics(&db, &opts).expect("run delivery-metrics");

    let row = rows
        .iter()
        .find(|r| r.metric == "branch_duration_hours")
        .expect("branch_duration_hours row present");

    // Two merges: 48 h (branch1) and 4 h (branch2).
    // PERCENTILE_CONT(0.5) on a 2-element set = arithmetic mean = 26.0.
    assert_eq!(row.n, 2, "exactly 2 merge units");
    assert!(
        (row.p50 - 26.0_f64).abs() < 0.5,
        "branch_duration p50 expected ~26 h, got {:.2}",
        row.p50
    );
    assert!(row.p75 >= row.p50, "p75 >= p50");
    assert!(row.p90 >= row.p75, "p90 >= p75");
}

#[test]
fn lead_proxy_p50_is_24_hours() {
    let fixture = codelore_lib::test_support::delivery_repo::build();
    let repo = GixRepo::open(fixture.dir.path()).expect("open repo");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = base_opts(fixture.dir.path());
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_delivery_metrics(&db, &opts).expect("run delivery-metrics");

    let row = rows
        .iter()
        .find(|r| r.metric == "lead_proxy_hours")
        .expect("lead_proxy_hours row present");

    // Two non-merge commits have a 24 h author→committer gap; all others are 0
    // and filtered out by the `> 0` predicate.
    assert_eq!(
        row.n, 2,
        "exactly 2 commits with positive author→committer gap"
    );
    assert!(
        (row.p50 - 24.0_f64).abs() < 0.5,
        "lead_proxy p50 expected 24 h, got {:.2}",
        row.p50
    );
}

#[test]
fn rework_pct_is_positive() {
    let fixture = codelore_lib::test_support::delivery_repo::build();
    let repo = GixRepo::open(fixture.dir.path()).expect("open repo");
    let db = FactsDb::new_in_memory().expect("db");
    // rework_window_days=365: all 2026 fixture commits fall within 365d of HEAD
    // (2026-04-21), so the day-5→day-8 rework signal is captured.
    // With the default 21d window the signal is absent (rework events are
    // ~110 days before HEAD), which is the correct behaviour: window-anchored
    // to HEAD means old rework doesn't inflate current metrics.
    let opts = Options {
        repo_path: fixture.dir.path().to_path_buf(),
        include_merges: true,
        min_revs: 1,
        rework_window_days: 365,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_delivery_metrics(&db, &opts).expect("run delivery-metrics");

    let row = rows
        .iter()
        .find(|r| r.metric == "rework_pct")
        .expect("rework_pct row present");

    // Day 5: Alice adds 2 lines to rework.rs (new_lines=2).
    // Day 8: Bob deletes those 2 lines (within the 3-day pair gap ≤ 365d window).
    // Overlap > 0 → rework_pct > 0.
    // Exact value is not hand-computable because the denominator is the total
    // new_lines across ALL hunks in the 365d window (many other commits add lines
    // too), but the signal must be strictly positive.
    assert!(
        row.p50 > 0.0,
        "rework_pct must be positive due to the day-5→day-8 rework signal; got {:.4}",
        row.p50
    );
    assert!(row.n > 0, "rework_pct n (pair count) must be positive");
}

/// Build a tiny repo where one 10-line file is created and then fully
/// rewritten four times in quick succession, so the identical 10-line range
/// is independently overlapped by every later rewrite within the rework
/// window (a hot file rewritten repeatedly).
///
/// The initial `c0` file-add carries NO hunk row — the ingest only records
/// hunks for modifications (pure adds/deletes carry empty hunks, see
/// `repo::gix_repo`). The four rewrite commits `c1..c4` are modifications,
/// each producing one full-range replace hunk `old=1..11, new=1..11`
/// (`new_lines=10`), all at the same line numbers. That yields four windowed
/// hunks and `C(4,2) = 6` forward-in-time pairs, each with a full 10-line
/// overlap.
///
/// Regression fixture for the rework-overlap cap: before the fix,
/// `SUM(overlap)` counts each hunk's lines once per later reworking partner
/// → 6 × 10 = 60 against a denominator of 40 (`4 × 10` `new_lines`) → 150%,
/// which exceeds the logically possible maximum of 100%.
fn build_rework_multi_partner_repo() -> tempfile::TempDir {
    use std::fmt::Write as _;
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
        "2026-01-01T00:00:00Z",
        &["init", "-b", "main", "--quiet"],
    );
    run(
        path,
        "2026-01-01T00:00:00Z",
        &["config", "user.email", "t@t"],
    );
    run(
        path,
        "2026-01-01T00:00:00Z",
        &["config", "user.name", "Tiny"],
    );

    let file = path.join("rework.txt");
    let write_version = |n: u32| {
        // 10 distinct lines; every line changes between versions so each
        // rewrite is a single full-range replace hunk.
        let mut content = String::new();
        for i in 1..=10u32 {
            writeln!(content, "v{n}-line-{i}").expect("format fixture line");
        }
        std::fs::write(&file, content).expect("write fixture file");
    };

    // c0 (Jan 1): create the file. A pure add carries NO hunk row, so it is
    // not one of the reworking hunks — it only establishes the baseline that
    // the later rewrites modify.
    write_version(0);
    run(path, "2026-01-01T00:00:00Z", &["add", "rework.txt"]);
    run(
        path,
        "2026-01-01T00:00:00Z",
        &["commit", "-m", "c0 add", "--quiet"],
    );

    // c1..c4 (Jan 2..5): each rewrites every line, so git diff reports one
    // full-range replace hunk `old=1..11, new=1..11` (new_lines=10) at the
    // same line numbers. Four modification hunks → C(4,2)=6 forward pairs.
    for (day, version) in [(2u32, 1u32), (3, 2), (4, 3), (5, 4)] {
        write_version(version);
        let date = format!("2026-01-0{day}T00:00:00Z");
        let msg = format!("c{version} rewrite");
        run(path, &date, &["commit", "-am", &msg, "--quiet"]);
    }

    dir
}

#[test]
fn rework_pct_capped_at_100_with_multiple_reworking_partners() {
    let fixture = build_rework_multi_partner_repo();
    let repo = GixRepo::open(fixture.path()).expect("open repo");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: fixture.path().to_path_buf(),
        include_merges: true,
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_delivery_metrics(&db, &opts).expect("run delivery-metrics");

    let row = rows
        .iter()
        .find(|r| r.metric == "rework_pct")
        .expect("rework_pct row present");

    // Four modification hunks (c1..c4), all covering the identical 10-line
    // range. Every one of the C(4,2) = 6 forward-in-time pairs has
    // overlap = 10 lines.
    //
    // Pre-fix: SUM(overlap) = 60, denominator (total new_lines) = 40 →
    // 150%, i.e. > 100%.
    //
    // Post-fix: each earlier hunk's total forward overlap is capped at its
    // own new_lines before summing: c1 → min(10+10+10, 10) = 10;
    // c2 → min(10+10, 10) = 10; c3 → min(10, 10) = 10; c4 → no later partner
    // → 0. Total = 30, so rework_pct = 100 × 30 / 40 = 75.0.
    assert_eq!(row.n, 6, "6 forward-looking hunk pairs (C(4,2))");
    assert!(
        row.p50 <= 100.0 + 1e-9,
        "rework_pct must never exceed 100%; got {:.4}",
        row.p50
    );
    assert!(
        (row.p50 - 75.0_f64).abs() < 0.5,
        "rework_pct expected ~75.0 after capping per-added-hunk overlap, got {:.4}",
        row.p50
    );
}

#[test]
fn batch_size_files_p50_is_one() {
    let fixture = codelore_lib::test_support::delivery_repo::build();
    let repo = GixRepo::open(fixture.dir.path()).expect("open repo");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = base_opts(fixture.dir.path());
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_delivery_metrics(&db, &opts).expect("run delivery-metrics");

    let row = rows
        .iter()
        .find(|r| r.metric == "batch_size_files")
        .expect("batch_size_files row present");

    // Branch 1 touches only src/branch1.rs (1 file).
    // Branch 2 touches only src/branch2.rs (1 file).
    // PERCENTILE_CONT(0.5) on [1, 1] = 1.0.
    assert_eq!(row.n, 2, "exactly 2 merge units");
    assert!(
        (row.p50 - 1.0_f64).abs() < 0.5,
        "batch_size_files p50 expected 1.0, got {:.2}",
        row.p50
    );
}

#[test]
fn all_five_metrics_present() {
    let fixture = codelore_lib::test_support::delivery_repo::build();
    let repo = GixRepo::open(fixture.dir.path()).expect("open repo");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = base_opts(fixture.dir.path());
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_delivery_metrics(&db, &opts).expect("run delivery-metrics");

    let metric_names: Vec<&str> = rows.iter().map(|r| r.metric.as_str()).collect();
    for expected in &[
        "batch_size_files",
        "batch_size_loc",
        "branch_duration_hours",
        "rework_pct",
        "lead_proxy_hours",
    ] {
        assert!(
            metric_names.contains(expected),
            "metric '{expected}' missing from output; got: {metric_names:?}"
        );
    }

    // Each row must have a non-empty caveat string.
    for row in &rows {
        assert!(!row.caveat.is_empty(), "caveat empty for {}", row.metric);
    }
}

#[test]
fn no_merge_commits_returns_empty_or_graceful() {
    // Without include_merges the commit_parents table has no merge rows; the
    // analysis should return an empty Vec rather than error.
    let fixture = codelore_lib::test_support::delivery_repo::build();
    let repo = GixRepo::open(fixture.dir.path()).expect("open repo");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: fixture.dir.path().to_path_buf(),
        include_merges: false,
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    // Should not error — returns empty (no merge units to compute over).
    let rows = run_delivery_metrics(&db, &opts).expect("run delivery-metrics without merges");

    // Branch metrics require merge rows; expect empty or only lead_proxy.
    let branch_row = rows.iter().find(|r| r.metric == "branch_duration_hours");
    assert!(
        branch_row.is_none(),
        "branch_duration_hours should be absent without merge commits"
    );
}

#[test]
fn mainline_advance_stops_branch_walk_at_merge_base() {
    // Regression guard for the branch-walk overshoot: the `mainline_advance_repo`
    // fixture has a feature branch that stays open while main advances, so the
    // merge's first parent is newer than the merge base. The branch-side commits
    // touch only src/feature.rs (X on Jan 8, Y on Jan 9), merged Jan 10.
    //
    // Correct behaviour (single merge unit, n == 1):
    //   batch_size_files p50 = 1   (only src/feature.rs)
    //   branch_duration  p50 = 48  (Jan 10 10:00 − Jan 8 10:00)
    //
    // Before the mainline_reachable anti-join, the walk crossed the merge base
    // into mainline history (src/main.rs on Jan 5 and Jan 1), inflating
    // batch_size_files to 2 and branch_duration to 216 h.
    let fixture = codelore_lib::test_support::mainline_advance_repo::build();
    let repo = GixRepo::open(fixture.dir.path()).expect("open repo");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = base_opts(fixture.dir.path());
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_delivery_metrics(&db, &opts).expect("run delivery-metrics");

    let files = rows
        .iter()
        .find(|r| r.metric == "batch_size_files")
        .expect("batch_size_files row present");
    assert_eq!(files.n, 1, "exactly one merge unit");
    assert!(
        (files.p50 - 1.0_f64).abs() < 0.5,
        "batch_size_files must be 1 (only src/feature.rs); crossing the merge \
         base would pull in src/main.rs and inflate to 2. got {:.2}",
        files.p50
    );

    let dur = rows
        .iter()
        .find(|r| r.metric == "branch_duration_hours")
        .expect("branch_duration_hours row present");
    assert_eq!(dur.n, 1, "exactly one merge unit");
    assert!(
        (dur.p50 - 48.0_f64).abs() < 0.5,
        "branch_duration must be 48 h (Jan 10 − Jan 8); crossing the merge base \
         would push MIN(date) back to Jan 1 and inflate to 216 h. got {:.2}",
        dur.p50
    );
}
