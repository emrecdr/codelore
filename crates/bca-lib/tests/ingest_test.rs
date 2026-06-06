use bca_lib::Options;
use bca_lib::facts::FactsDb;
use bca_lib::repo::GixRepo;

#[test]
fn ingest_tiny_repo_writes_5_commits() {
    let tiny = bca_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");

    let opts = Options::default();
    let n = db.ingest(&repo, &opts).expect("ingest");
    assert_eq!(n.commits_ingested, 5);

    let count: String = db
        .query_one_value("SELECT CAST(COUNT(*) AS TEXT) FROM commits")
        .expect("count");
    assert_eq!(count, "5");
}

#[test]
fn ingest_populates_complexity_for_tier1_files() {
    let tiny = bca_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");

    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let entity_count: String = db
        .query_one_value("SELECT CAST(COUNT(*) AS TEXT) FROM entities WHERE path = 'src/main.rs'")
        .expect("entity count query");
    let n: u32 = entity_count.parse().unwrap();
    assert!(n >= 1, "expected ≥1 entity for src/main.rs, got {n}");

    let metric_count: String = db
        .query_one_value(
            "SELECT CAST(COUNT(*) AS TEXT) FROM complexity_metrics WHERE path = 'src/main.rs'",
        )
        .expect("metric count query");
    let m: u32 = metric_count.parse().unwrap();
    assert!(
        m >= 1,
        "expected ≥1 complexity row for src/main.rs, got {m}"
    );
}
