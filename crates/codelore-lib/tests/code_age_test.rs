use codelore_lib::Options;
use codelore_lib::analyses::code_age::run_code_age;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;

#[test]
fn code_age_for_tiny_repo() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_code_age(&db, &opts).expect("run");
    assert!(!rows.is_empty(), "should produce at least 1 row");

    // All ages must be >= 0
    for row in &rows {
        assert!(
            row.age_months >= 0,
            "age must be >= 0, got {} for {}",
            row.age_months,
            row.path
        );
    }

    // tiny_repo commits happen "now" → age in months should be 0
    let main_row = rows
        .iter()
        .find(|r| r.path == "src/main.rs")
        .expect("main.rs");
    assert!(
        main_row.age_months <= 1,
        "tiny_repo just built; main.rs age should be ≤1 month, got {}",
        main_row.age_months
    );
}
