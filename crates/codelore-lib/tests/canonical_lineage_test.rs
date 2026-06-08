//! Validation of the recursive rename-lineage CTE.
//!
//! The `differential_repo` fixture renames `src/old_name.rs` → `src/new_name.rs`
//! once (commit `546f33a`) and then `src/new_name.rs` gets one more
//! revision (`later change to src/new_name.rs`). The pre-rename file had
//! at least one prior commit too. Without canonical lineage, `revisions`
//! reports TWO entities (`old_name.rs` and `new_name.rs`) with split
//! counts. With lineage, they merge under `new_name.rs`.

use codelore_lib::Options;
use codelore_lib::analyses::revisions::run_revisions;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;

#[test]
fn rename_history_merges_under_canonical_path_when_lineage_on() {
    let diff = codelore_lib::test_support::differential_repo::build();
    let repo = GixRepo::open(diff.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");

    let opts_on = Options {
        repo_path: diff.dir.path().to_path_buf(),
        min_revs: 1,
        use_canonical_lineage: true,
        ..Options::default()
    };
    db.ingest(&repo, &opts_on).expect("ingest");
    let rows_on = run_revisions(&db, &opts_on).expect("revisions");

    // With lineage on: the OLD path must NOT appear in the output
    // because all its revisions have been folded into the new path.
    let old_appears = rows_on.iter().any(|(path, _)| path.contains("old_name"));
    assert!(
        !old_appears,
        "old_name.rs should NOT appear when canonical_lineage is on; got rows: {rows_on:?}"
    );
    // And the new path SHOULD appear with the merged history.
    let new_count = rows_on
        .iter()
        .find(|(p, _)| p.contains("new_name"))
        .map(|(_, n)| *n);
    assert!(
        new_count.is_some_and(|n| n >= 2),
        "new_name.rs should aggregate ≥2 revisions (pre-rename + post-rename); got: {rows_on:?}"
    );
}

#[test]
fn split_history_returns_when_lineage_off() {
    let diff = codelore_lib::test_support::differential_repo::build();
    let repo = GixRepo::open(diff.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");

    let opts_off = Options {
        repo_path: diff.dir.path().to_path_buf(),
        min_revs: 1,
        use_canonical_lineage: false,
        ..Options::default()
    };
    db.ingest(&repo, &opts_off).expect("ingest");
    let rows_off = run_revisions(&db, &opts_off).expect("revisions");

    // With lineage off: both paths appear (code-maat-parity behaviour).
    let old_appears = rows_off.iter().any(|(path, _)| path.contains("old_name"));
    let new_appears = rows_off.iter().any(|(path, _)| path.contains("new_name"));
    assert!(
        old_appears && new_appears,
        "both old_name and new_name should appear when canonical_lineage is off; got: {rows_off:?}"
    );
}
