use bca_lib::Options;
use bca_lib::analyses::code_health::run_code_health;
use bca_lib::facts::FactsDb;
use bca_lib::repo::GixRepo;

#[test]
fn code_health_for_tiny_repo() {
    let tiny = bca_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_code_health(&db, &opts).expect("run");
    assert!(!rows.is_empty(), "should produce ≥1 row");

    for row in &rows {
        assert!(
            row.score >= 0.0 && row.score <= 100.0,
            "score should be in [0, 100], got {} for {}",
            row.score,
            row.path
        );
        assert!(
            row.cognitive >= 0.0,
            "cognitive should be >= 0, got {} for {}",
            row.cognitive,
            row.path
        );
    }
}

#[test]
fn code_health_ranks_least_healthy_first() {
    // Convention: ORDER BY score ASC — least healthy first (these are the
    // ones a developer should look at).
    let tiny = bca_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_code_health(&db, &opts).expect("run");
    // Confirm ascending order
    for w in rows.windows(2) {
        assert!(
            w[0].score <= w[1].score,
            "expected ascending score order, got {} > {}",
            w[0].score,
            w[1].score
        );
    }
}
