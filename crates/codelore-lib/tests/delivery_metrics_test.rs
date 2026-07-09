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
