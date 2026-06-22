//! End-to-end coverage of `run_lead_time` against an ingested `FactsDb`.
//!
//! lead-time is a DORA metric (review wait between author-time and
//! committer-time). Today's `commits` schema carries only committer
//! date, so every row reports 0 lead-time — but the SQL must still
//! execute cleanly + the row shape must hold. The audit cycle flagged that a
//! schema rename or typo would only surface at customer runtime; this
//! test pins the runtime contract.

use codelore_lib::Options;
use codelore_lib::analyses::lead_time::run_lead_time;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;

#[test]
fn lead_time_emits_one_row_per_commit() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_lead_time(&db, &opts).expect("run lead-time");

    // `tiny_repo` ships 5 commits; lead-time emits one row per commit.
    assert_eq!(rows.len(), 5, "expected 5 lead-time rows for tiny_repo");

    for row in &rows {
        // Schema invariants — `rev` is a SHA, never empty.
        assert!(!row.rev.is_empty(), "rev must be populated");
        assert_eq!(
            row.rev.len(),
            40,
            "rev should be the 40-char SHA, got {} chars",
            row.rev.len()
        );
        // Author date and committer date must both be ISO strings; we
        // don't assert equality (a future schema bump will diverge
        // them once `author_date` lands) but both must be non-empty.
        assert!(!row.author_date.is_empty(), "author_date populated");
        assert!(!row.committer_date.is_empty(), "committer_date populated");
        assert!(
            !row.canonical_author.is_empty(),
            "canonical_author populated"
        );
    }
}
