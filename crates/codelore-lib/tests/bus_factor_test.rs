//! End-to-end coverage of `run_bus_factor` against an ingested `FactsDb`.
//!
//! The audit cycle (F140) called out that bus-factor had a row struct,
//! a unit test, but no integration test against a real ingested
//! repository — so SQL typos or schema renames would surface only at
//! customer runtime. This test exercises the full path through
//! `run_bus_factor`, asserting both shape invariants and the
//! Filatov-2010 bus-factor semantic (smaller = more concentrated).

use codelore_lib::Options;
use codelore_lib::analyses::bus_factor::run_bus_factor;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;

#[test]
fn bus_factor_on_tiny_repo_concentrates_to_one_author() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_bus_factor(&db, &opts).expect("run bus-factor");

    // `tiny_repo` has a single author across all commits, so every
    // module's bus_factor must be exactly 1 (one author covers
    // ≥80% of commits by construction). The presence of at least
    // one row also proves the SQL didn't return an empty result
    // set silently.
    assert!(!rows.is_empty(), "expected at least one module row");
    for row in &rows {
        assert_eq!(
            row.bus_factor, 1,
            "single-author `tiny_repo` must yield bus_factor=1 for module {}",
            row.module,
        );
        assert!(
            row.total_commits > 0,
            "module {} reported total_commits=0",
            row.module,
        );
    }
}
