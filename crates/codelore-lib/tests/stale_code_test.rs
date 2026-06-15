//! End-to-end coverage of `run_stale_code` against an ingested `FactsDb`.
//!
//! Stale-code uses two fixed thresholds today (≥12 months untouched,
//! cognitive ≤ 5). The `tiny_repo` fixture's commits are all
//! present-day, so stale-code legitimately returns an empty result.
//! The point of this test is to exercise the SQL end-to-end against a
//! real `DuckDB` — a typo or schema rename (the exact regression class
//! that bit stale-code's `DATE_DIFF` binder error in the validation
//! pass) would surface here at CI time instead of at customer runtime.

use codelore_lib::Options;
use codelore_lib::analyses::stale_code::run_stale_code;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;

#[test]
fn stale_code_executes_cleanly_on_tiny_repo() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    // `tiny_repo` has only fresh commits → stale-code returns empty.
    // The SQL must still execute cleanly; an empty `Vec` is the
    // success signal, not a `Result::Err`.
    let rows = run_stale_code(&db, &opts).expect("run stale-code");
    assert!(
        rows.is_empty(),
        "tiny_repo has only fresh commits — expected empty stale-code result, got {} rows",
        rows.len(),
    );
}
