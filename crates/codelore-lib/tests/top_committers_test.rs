//! `top-committers` analysis tests. This analysis is the per-author
//! global leaderboard (previously the behaviour of the `authors`
//! analysis before PAR-1 separated the per-entity Bird et al. risk
//! signal into its own analysis with the canonical `authors` name).

use codelore_lib::Options;
use codelore_lib::analyses::top_committers::run_top_committers;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;

#[test]
fn top_committers_groups_by_canonical_author_and_sorts_desc() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_top_committers(&db, &opts).expect("run top-committers");
    // tiny_repo: 5 commits, 1 author ("Tiny" / tiny@example.com).
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].commits, 5);
    assert!(!rows[0].is_bot, "Tiny is not a bot");
}

#[test]
fn top_committers_against_differential_fixture() {
    let fixture = codelore_lib::test_support::differential_repo::build();
    let repo = GixRepo::open(fixture.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: fixture.dir.path().to_path_buf(),
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_top_committers(&db, &opts).expect("run top-committers");
    assert_eq!(
        rows.len(),
        4,
        "expected 4 distinct canonical authors with merges filtered (3 humans + 1 bot), got {}: {:?}",
        rows.len(),
        rows.iter().map(|r| &r.author).collect::<Vec<_>>()
    );
    let names: std::collections::HashSet<&str> = rows.iter().map(|r| r.author.as_str()).collect();
    assert!(names.contains("canonical-alice@example.com"));
    assert!(names.contains("canonical-bob@example.com"));
    assert!(names.contains("carol@example.com"));
    // Sorted desc by commit count
    for w in rows.windows(2) {
        assert!(
            w[0].commits >= w[1].commits,
            "authors must be sorted by commits desc; got {} then {}",
            w[0].commits,
            w[1].commits
        );
    }
}

#[test]
fn top_committers_with_merges_includes_merge_committer() {
    let fixture = codelore_lib::test_support::differential_repo::build();
    let repo = GixRepo::open(fixture.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: fixture.dir.path().to_path_buf(),
        include_merges: true,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_top_committers(&db, &opts).expect("run top-committers");
    assert_eq!(rows.len(), 5);
    let names: std::collections::HashSet<&str> = rows.iter().map(|r| r.author.as_str()).collect();
    assert!(names.contains("noop@example.com"));
}

#[test]
fn top_committers_rows_limit_caps_output() {
    let fixture = codelore_lib::test_support::differential_repo::build();
    let repo = GixRepo::open(fixture.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: fixture.dir.path().to_path_buf(),
        rows_limit: Some(2),
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_top_committers(&db, &opts).expect("run top-committers");
    assert!(rows.len() <= 2, "rows_limit should cap output");
}

/// First / last commit dates are populated and bracket the fixture's
/// commit history (calendar-date precision).
#[test]
fn top_committers_emits_first_and_last_commit_dates() {
    let fixture = codelore_lib::test_support::differential_repo::build();
    let repo = GixRepo::open(fixture.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: fixture.dir.path().to_path_buf(),
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_top_committers(&db, &opts).expect("run top-committers");
    for row in &rows {
        assert!(
            !row.first_commit.is_empty(),
            "first_commit must be populated for {}",
            row.author
        );
        assert!(
            !row.last_commit.is_empty(),
            "last_commit must be populated for {}",
            row.author
        );
        assert!(
            row.first_commit <= row.last_commit,
            "first_commit ({}) must be <= last_commit ({}) for {}",
            row.first_commit,
            row.last_commit,
            row.author
        );
    }
}
