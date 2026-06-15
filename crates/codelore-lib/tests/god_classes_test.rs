//! End-to-end coverage of `run_god_classes` against an ingested `FactsDb`.
//!
//! God-classes intersects high cognitive complexity with high coupling
//! (`fan_in` + `fan_out` via the imports table). `tiny_repo` has trivial
//! code (no real coupling, no high cognitive), so the analysis
//! legitimately returns empty — the test pins the SQL/schema contract
//! via successful execution.

use codelore_lib::Options;
use codelore_lib::analyses::god_classes::run_god_classes;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;

#[test]
fn god_classes_executes_cleanly_on_tiny_repo() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_god_classes(&db, &opts).expect("run god-classes");

    // `tiny_repo` has trivial code → no god-classes. The SQL must still
    // execute cleanly + return a (possibly empty) Vec.
    for row in &rows {
        // Shape invariants — every row that DOES surface must carry a
        // populated path and non-negative metrics.
        assert!(!row.path.is_empty(), "god-class row has empty path");
        assert!(
            row.cognitive >= 0.0,
            "cognitive must be non-negative for {}",
            row.path,
        );
        assert!(
            row.god_score >= 0.0,
            "god_score must be non-negative for {}",
            row.path,
        );
    }
}
