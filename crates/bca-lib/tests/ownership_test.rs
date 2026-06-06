use bca_lib::Options;
use bca_lib::analyses::ownership::run_ownership;
use bca_lib::facts::FactsDb;
use bca_lib::repo::GixRepo;

#[test]
fn ownership_single_author_has_zero_fragmentation() {
    let tiny = bca_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_ownership(&db, &opts).expect("run");
    assert!(!rows.is_empty(), "ownership should produce ≥1 row");
    for row in &rows {
        // single author → HHI = 1, FV = 0
        assert!(
            row.fractal_value < 1e-9,
            "single-author file should have FV ≈ 0, got {} for {}",
            row.fractal_value,
            row.path
        );
        assert_eq!(row.main_author, "tiny@example.com", "tiny_repo author");
    }
}
