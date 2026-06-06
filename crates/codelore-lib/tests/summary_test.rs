use codelore_lib::Options;
use codelore_lib::analyses::summary::run_summary;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;

#[test]
fn summary_for_tiny_repo() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_summary(&db, &opts).expect("run");
    assert_eq!(rows.len(), 4, "summary should produce exactly 4 rows");

    let commits = rows.iter().find(|r| r.metric == "commits").unwrap();
    assert_eq!(commits.value, 5, "tiny_repo has 5 commits");

    let authors = rows.iter().find(|r| r.metric == "authors").unwrap();
    assert_eq!(authors.value, 1, "tiny_repo has 1 author");
}
