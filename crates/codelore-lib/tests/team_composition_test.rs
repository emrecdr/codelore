//! Tests for the `team-composition` analysis.
//!
//! Fixture legend:
//!
//! - **`tiny_repo`** — single author. All 3 properties are deterministic:
//!   `bucket = "onboarded"` (span < 90 d), `veteran_breadth_ok = false`
//!   (only author IS the core set; they meet the median trivially BUT the
//!   span never reaches 365 d), `onboarding_weeks = NULL` (founder).
//!
//! - **`delivery_repo`** — Alice / Bob / Carol. Project spans Jan-1 to Apr-21
//!   (110 days). All three first commits are within the first 12 weeks
//!   (founder cutoff = Jan-1 + 84 d = Mar-26) → all `onboarding_weeks = NULL`.
//!   Alice span = 110 d → "experienced". Bob span = 58 d → "onboarded".
//!   Carol span = 64 d → "onboarded". Nobody reaches 365 d → no veteran.
//!   Summary row must appear with `author = "__summary__"`.

use codelore_lib::Options;
use codelore_lib::analyses::team_composition::run_team_composition;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::gix_repo::GixRepo;

fn ingest(repo_path: &std::path::Path) -> FactsDb {
    let db = FactsDb::new_in_memory().expect("in-memory db");
    let repo = GixRepo::open(repo_path).expect("open repo");
    let opts = Options {
        repo_path: repo_path.to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");
    db
}

// ── 1. tiny_repo ─────────────────────────────────────────────────────────────

#[test]
fn tiny_repo_runs_without_error() {
    let fixture = codelore_lib::test_support::tiny_repo::build();
    let db = ingest(fixture.dir.path());
    let opts = Options {
        repo_path: fixture.dir.path().to_path_buf(),
        window_days: 365,
        min_revs: 1,
        ..Options::default()
    };
    let rows = run_team_composition(&db, &opts).expect("run team-composition");
    // Summary row always present when there is ≥1 author.
    assert!(
        rows.iter().any(|r| r.author == "__summary__"),
        "summary row must be present"
    );
}

#[test]
fn tiny_repo_single_author_is_onboarded_bucket() {
    let fixture = codelore_lib::test_support::tiny_repo::build();
    let db = ingest(fixture.dir.path());
    let opts = Options {
        repo_path: fixture.dir.path().to_path_buf(),
        window_days: 365,
        min_revs: 1,
        ..Options::default()
    };
    let rows = run_team_composition(&db, &opts).expect("run team-composition");
    let author_rows: Vec<_> = rows.iter().filter(|r| r.author != "__summary__").collect();
    for row in &author_rows {
        assert!(
            row.bucket == "onboarded" || row.bucket == "experienced",
            "tiny_repo spans days, not years; bucket must be onboarded or experienced, got {:?} for {}",
            row.bucket,
            row.author,
        );
    }
}

// ── 2. delivery_repo ─────────────────────────────────────────────────────────

#[test]
fn delivery_repo_returns_three_author_rows_plus_summary() {
    let fixture = codelore_lib::test_support::delivery_repo::build();
    let db = ingest(fixture.dir.path());
    let opts = Options {
        repo_path: fixture.dir.path().to_path_buf(),
        window_days: 90,
        min_revs: 1,
        ..Options::default()
    };
    let rows = run_team_composition(&db, &opts).expect("run team-composition");
    let author_rows: Vec<_> = rows.iter().filter(|r| r.author != "__summary__").collect();
    assert_eq!(
        author_rows.len(),
        3,
        "delivery_repo has 3 canonical authors; got {} author rows",
        author_rows.len(),
    );
    assert!(
        rows.iter().any(|r| r.author == "__summary__"),
        "summary row must be present"
    );
}

#[test]
fn delivery_repo_alice_is_experienced() {
    // Alice: first=2026-01-01, last=2026-04-21 → span=110 d → "experienced"
    let fixture = codelore_lib::test_support::delivery_repo::build();
    let db = ingest(fixture.dir.path());
    let opts = Options {
        repo_path: fixture.dir.path().to_path_buf(),
        window_days: 90,
        min_revs: 1,
        ..Options::default()
    };
    let rows = run_team_composition(&db, &opts).expect("run team-composition");
    let alice = rows
        .iter()
        .find(|r| r.author.contains("alice"))
        .expect("alice row must be present");
    assert_eq!(
        alice.bucket, "experienced",
        "alice span=110 d → experienced; got {:?}",
        alice.bucket,
    );
    assert!(
        alice.tenure_days >= 100 && alice.tenure_days <= 120,
        "alice tenure_days must be around 110, got {}",
        alice.tenure_days,
    );
}

#[test]
fn delivery_repo_no_veterans() {
    // No author has span ≥ 365 d in delivery_repo.
    let fixture = codelore_lib::test_support::delivery_repo::build();
    let db = ingest(fixture.dir.path());
    let opts = Options {
        repo_path: fixture.dir.path().to_path_buf(),
        window_days: 90,
        min_revs: 1,
        ..Options::default()
    };
    let rows = run_team_composition(&db, &opts).expect("run team-composition");
    for row in rows.iter().filter(|r| r.author != "__summary__") {
        assert_ne!(
            row.bucket, "veteran",
            "no author spans 365 d in delivery_repo; got veteran for {}",
            row.author,
        );
    }
}

#[test]
fn delivery_repo_all_founders_have_null_onboarding_weeks() {
    // All three authors' first commits are within the first 12 weeks of the
    // project (project start = Jan-1, cutoff = Jan-1 + 84 d = Mar-26;
    // alice=Jan-1, bob=Jan-8, carol=Jan-17 — all before Mar-26).
    let fixture = codelore_lib::test_support::delivery_repo::build();
    let db = ingest(fixture.dir.path());
    let opts = Options {
        repo_path: fixture.dir.path().to_path_buf(),
        window_days: 90,
        min_revs: 1,
        ..Options::default()
    };
    let rows = run_team_composition(&db, &opts).expect("run team-composition");
    for row in rows.iter().filter(|r| r.author != "__summary__") {
        assert_eq!(
            row.onboarding_weeks, None,
            "all delivery_repo authors are founders; onboarding_weeks must be NULL for {}",
            row.author,
        );
    }
}

#[test]
fn delivery_repo_summary_bucket_string_contains_pcts() {
    let fixture = codelore_lib::test_support::delivery_repo::build();
    let db = ingest(fixture.dir.path());
    let opts = Options {
        repo_path: fixture.dir.path().to_path_buf(),
        window_days: 90,
        min_revs: 1,
        ..Options::default()
    };
    let rows = run_team_composition(&db, &opts).expect("run team-composition");
    let summary = rows
        .iter()
        .find(|r| r.author == "__summary__")
        .expect("summary row must be present");
    // The bucket field for the summary row is e.g.
    // "onboarded=66.7% experienced=33.3% veteran=0.0%"
    assert!(
        summary.bucket.contains("onboarded=") && summary.bucket.contains("experienced="),
        "summary bucket must contain onboarded and experienced keys; got {:?}",
        summary.bucket,
    );
}
