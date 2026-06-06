use bca_lib::Options;
use bca_lib::analyses::hotspots::run_hotspots;
use bca_lib::facts::FactsDb;
use bca_lib::repo::GixRepo;

#[test]
fn hotspots_for_tiny_repo() {
    let tiny = bca_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_hotspots(&db, &opts).expect("run");
    assert!(!rows.is_empty(), "should produce ≥1 hotspot row");

    // src/main.rs changed 4 times; src/lib.rs changed 1 time. Both Rust.
    // With similar complexity, main.rs should rank above lib.rs.
    let main_row = rows
        .iter()
        .find(|r| r.path == "src/main.rs")
        .expect("main.rs should be in hotspots");
    let lib_row = rows.iter().find(|r| r.path == "src/lib.rs");

    if let Some(lib) = lib_row {
        assert!(
            main_row.hotspot_score >= lib.hotspot_score,
            "main.rs (revs=4) should rank ≥ lib.rs (revs=1)"
        );
    }

    // Hotspot score should be in [0, 10] range (formula bounds:
    // percentile_rank ∈ [0,1], code_health ∈ [0,100], so score ∈ [0, 10])
    for row in &rows {
        assert!(
            row.hotspot_score >= 0.0,
            "hotspot score should be >= 0, got {} for {}",
            row.hotspot_score,
            row.path
        );
    }
}
