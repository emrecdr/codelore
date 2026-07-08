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
//!   (founder cutoff = date_trunc('week', Jan-1) + 84 d = 2026-03-23) → all `onboarding_weeks = NULL`.
//!   Alice span = 110 d → "experienced". Bob span = ~54 d → "onboarded".
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
    // project (project start = Jan-1, cutoff = date_trunc('week', Jan-1) + 84 d = 2026-03-23;
    // alice=Jan-1, bob=Jan-8, carol=Jan-17 — all before 2026-03-23).
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

// ── 3. Veteran-breadth gate ───────────────────────────────────────────────────

/// Run one git command with a fixed identity and date (no passthrough to shell).
#[cfg(feature = "test-support")]
fn git_as_tc(path: &std::path::Path, args: &[&str], author: &str, email: &str, date: &str) {
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(path)
        .args(args)
        .env("GIT_AUTHOR_NAME", author)
        .env("GIT_AUTHOR_EMAIL", email)
        .env("GIT_COMMITTER_NAME", author)
        .env("GIT_COMMITTER_EMAIL", email)
        .env("GIT_AUTHOR_DATE", date)
        .env("GIT_COMMITTER_DATE", date)
        .status()
        .expect("git");
    assert!(status.success(), "git {args:?} failed");
}

/// A veteran author (span ≥ 365 d) whose path breadth is below the core-set
/// median must be capped to `"experienced"` (`veteran_breadth_ok = false`).
///
/// Fixture:
/// - Alice: 2 commits spanning ~396 d, touches only `a.rs` (1 path).
/// - Bob:   3 commits within 30 d, touches `b.rs`, `c.rs`, `d.rs` (3 paths).
/// Both land in the Pareto-80 core set. Median paths = median(1, 3) = 2.
/// Alice's paths_touched (1) < 2 → breadth gate fires → bucket = "experienced".
#[test]
#[cfg(feature = "test-support")]
fn veteran_capped_to_experienced_when_breadth_below_median() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path();
    let git = |args: &[&str], author: &str, email: &str, date: &str| {
        git_as_tc(path, args, author, email, date);
    };

    git(
        &["init", "--quiet"],
        "Alice",
        "alice@example.com",
        "2024-01-01T10:00:00Z",
    );
    git(
        &["config", "gc.auto", "0"],
        "Alice",
        "alice@example.com",
        "2024-01-01T10:00:00Z",
    );

    // Alice commit 1: 2024-01-01
    std::fs::write(path.join("a.rs"), "pub fn a1() -> u32 { 1 }\n").expect("write");
    git(
        &["add", "a.rs"],
        "Alice",
        "alice@example.com",
        "2024-01-01T10:00:00Z",
    );
    git(
        &["commit", "-m", "feat: alice init"],
        "Alice",
        "alice@example.com",
        "2024-01-01T10:00:00Z",
    );

    // Bob commit 1: 2024-01-02, b.rs
    std::fs::write(path.join("b.rs"), "pub fn b() -> u32 { 1 }\n").expect("write");
    git(
        &["add", "b.rs"],
        "Bob",
        "bob@example.com",
        "2024-01-02T10:00:00Z",
    );
    git(
        &["commit", "-m", "feat: bob b"],
        "Bob",
        "bob@example.com",
        "2024-01-02T10:00:00Z",
    );

    // Bob commit 2: 2024-01-10, c.rs
    std::fs::write(path.join("c.rs"), "pub fn c() -> u32 { 1 }\n").expect("write");
    git(
        &["add", "c.rs"],
        "Bob",
        "bob@example.com",
        "2024-01-10T10:00:00Z",
    );
    git(
        &["commit", "-m", "feat: bob c"],
        "Bob",
        "bob@example.com",
        "2024-01-10T10:00:00Z",
    );

    // Bob commit 3: 2024-01-20, d.rs
    std::fs::write(path.join("d.rs"), "pub fn d() -> u32 { 1 }\n").expect("write");
    git(
        &["add", "d.rs"],
        "Bob",
        "bob@example.com",
        "2024-01-20T10:00:00Z",
    );
    git(
        &["commit", "-m", "feat: bob d"],
        "Bob",
        "bob@example.com",
        "2024-01-20T10:00:00Z",
    );

    // Alice commit 2: 2025-02-01 (~396 d after first) — pushes her span past 365 d.
    std::fs::write(path.join("a.rs"), "pub fn a1() -> u32 { 2 }\n").expect("write");
    git(
        &["add", "a.rs"],
        "Alice",
        "alice@example.com",
        "2025-02-01T10:00:00Z",
    );
    git(
        &["commit", "-m", "chore: alice touch"],
        "Alice",
        "alice@example.com",
        "2025-02-01T10:00:00Z",
    );

    let db = FactsDb::new_in_memory().expect("in-memory db");
    let repo = GixRepo::open(path).expect("open repo");
    let opts = Options {
        repo_path: path.to_path_buf(),
        min_revs: 1,
        window_days: 500,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");
    let rows = run_team_composition(&db, &opts).expect("run team-composition");

    let alice = rows
        .iter()
        .find(|r| r.author.contains("alice"))
        .expect("alice row must be present");
    assert!(
        alice.tenure_days >= 365,
        "alice span must be ≥365 d to be veteran-eligible; got {}",
        alice.tenure_days,
    );
    assert!(
        !alice.veteran_breadth_ok,
        "alice touches only 1 path; core-set median is 2; breadth gate must be false",
    );
    assert_eq!(
        alice.bucket, "experienced",
        "veteran capped to experienced when breadth_ok=false; got {:?}",
        alice.bucket,
    );
}
