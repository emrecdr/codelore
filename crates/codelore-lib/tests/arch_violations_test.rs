//! End-to-end coverage of `run_arch_violations` against an ingested
//! `FactsDb`. The analysis is opt-in via `.codelore-arch-rules.toml`;
//! `tiny_repo` has no rules file, so the run legitimately returns
//! empty. The test pins the runtime contract — SQL executes cleanly,
//! analysis returns empty rather than `Err`, no rules-file-missing
//! panic.

use codelore_lib::Options;
use codelore_lib::analyses::arch_violations::run_arch_violations;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;

#[test]
fn arch_violations_returns_empty_when_no_rules_file() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    // No .codelore-arch-rules.toml at the repo root → empty result.
    let rows = run_arch_violations(&db, &opts).expect("run arch-violations");
    assert!(
        rows.is_empty(),
        "no rules file should produce empty arch-violations result, got {} rows",
        rows.len(),
    );
}
