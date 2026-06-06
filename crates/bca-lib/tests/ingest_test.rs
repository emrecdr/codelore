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
