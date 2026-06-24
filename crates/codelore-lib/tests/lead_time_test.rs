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

#[test]
fn lead_time_breaks_ties_by_rev_under_limit() {
    // `tiny_repo`'s commits all carry equal author and committer
    // timestamps, so every row's `lead_time_seconds` is 0 — a fully
    // tied ranking. With a small LIMIT, which rows survive must be
    // deterministic: ordered by `rev` ASC as the tiebreaker, not by
    // scan order. The full unlimited result, truncated to the same
    // count, is the deterministic baseline.
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");

    let full_opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        ..Options::default()
    };
    db.ingest(&repo, &full_opts).expect("ingest");

    let all_rows = run_lead_time(&db, &full_opts).expect("run lead-time unlimited");
    assert!(all_rows.len() >= 3, "fixture needs enough tied rows");

    // All lead times tie at 0 for this fixture.
    assert!(
        all_rows.iter().all(|r| r.lead_time_seconds == 0),
        "tiny_repo commits should all tie at 0 lead-time seconds"
    );

    // Independent expectation: the revs that should survive a LIMIT 2
    // are the two smallest by `rev` ASC.
    let mut expected_revs: Vec<String> = all_rows.iter().map(|r| r.rev.clone()).collect();
    expected_revs.sort();
    expected_revs.truncate(2);

    let limited_opts = Options {
        rows_limit: Some(2),
        ..full_opts
    };
    let limited = run_lead_time(&db, &limited_opts).expect("run lead-time limited");
    let got_revs: Vec<String> = limited.iter().map(|r| r.rev.clone()).collect();

    assert_eq!(
        got_revs, expected_revs,
        "tied lead times must break by rev ASC so LIMIT is deterministic",
    );
}
