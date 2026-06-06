use bca_lib::Options;
use bca_lib::analyses::communication::run_communication;
use bca_lib::facts::FactsDb;
use bca_lib::repo::GixRepo;

#[test]
fn communication_for_tiny_repo_with_single_author() {
    let tiny = bca_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_shared_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_communication(&db, &opts).expect("run");
    // tiny_repo has 1 author → 0 pairs (self-pair excluded). Empty result is correct.
    assert!(
        rows.is_empty(),
        "single-author repo should produce no communication pairs, got {} rows",
        rows.len()
    );
}

#[test]
fn communication_row_shape() {
    use bca_lib::analyses::communication::CommunicationRow;
    let row = CommunicationRow {
        author_a: "a@b.com".into(),
        author_b: "c@d.com".into(),
        shared: 3,
        average: 5,
        strength: 60.0,
    };
    assert_eq!(row.author_a, "a@b.com");
    assert_eq!(row.shared, 3);
}
