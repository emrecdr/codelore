use codelore_lib::Options;
use codelore_lib::analyses::churn::{run_abs_churn, run_author_churn, run_entity_churn};
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;

fn fixture_db() -> (FactsDb, tempfile::TempDir) {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");
    (db, tiny.dir)
}

#[test]
fn abs_churn_groups_by_date() {
    let (db, _dir) = fixture_db();
    let opts = Options::default();
    let rows = run_abs_churn(&db, &opts).expect("run");
    assert!(!rows.is_empty(), "abs-churn should produce ≥1 row");
    // The gix LOC stub means added/deleted are 0; just check the date column has data
    for row in &rows {
        // date should be a non-empty ISO date string
        assert!(!row.date.is_empty());
    }
}

#[test]
fn author_churn_groups_by_canonical_author() {
    let (db, _dir) = fixture_db();
    let opts = Options::default();
    let rows = run_author_churn(&db, &opts).expect("run");
    assert!(
        !rows.is_empty(),
        "author-churn should have ≥1 row (tiny repo single author)"
    );
    // commits should be > 0 for any row
    for row in &rows {
        assert!(
            row.commits > 0,
            "commits should be > 0 for author '{}'",
            row.author
        );
    }
}

#[test]
fn entity_churn_groups_by_path() {
    let (db, _dir) = fixture_db();
    let opts = Options {
        min_revs: 1,
        ..Options::default()
    };
    let rows = run_entity_churn(&db, &opts).expect("run");
    assert!(!rows.is_empty(), "entity-churn should produce ≥1 row");
    // src/main.rs should appear with commits >= 4 (was edited in 4 commits in tiny_repo)
    let main_row = rows
        .iter()
        .find(|r| r.path == "src/main.rs")
        .expect("main.rs");
    assert!(
        main_row.commits >= 4,
        "src/main.rs should have ≥4 commits, got {}",
        main_row.commits
    );
}
