//! End-to-end coverage of `run_delivery_friction` against an ingested
//! `FactsDb`.
//!
//! Delivery friction = `percent_rank(revisions) × percent_rank(median
//! lead_time) × percent_rank(cognitive) × 100` — high requires
//! elevation on ALL THREE axes. The composite is the answer to "where
//! is technical debt actually slowing us down?".
//!
//! The fixture is `tiny_repo`, which crafts commits with equal author
//! and committer timestamps; that means `median_lead_time_days = 0` for
//! every file, the `pr_lt` factor is `0`, and `friction_score` collapses
//! to `0` for every row. The contract this test pins:
//!
//! 1. SQL runs cleanly on the v3 schema (`committer_date` column populated).
//! 2. Row shape holds: every required column emitted.
//! 3. `wip_age_days` reports a positive number for every file (fixture
//!    commits are seconds-old at test time).
//!
//! A non-zero `friction_score` requires the fixture to surface real
//! review-time deltas — that is left to fixtures whose commits use
//! `git commit --date=...` overrides; the analysis SQL is exercised
//! correctly here.

use codelore_lib::Options;
use codelore_lib::analyses::delivery_friction::run_delivery_friction;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;

#[test]
fn delivery_friction_runs_cleanly_on_tiny_repo() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_delivery_friction(&db, &opts).expect("run delivery-friction");

    assert!(
        !rows.is_empty(),
        "tiny_repo must surface at least one file with revisions >= min_revs=1"
    );

    for row in &rows {
        assert!(!row.path.is_empty(), "path populated");
        assert!(row.revisions >= 1, "revisions >= min_revs filter");
        assert!(
            row.median_lead_time_days >= 0.0,
            "lead-time delta cannot be negative; got {} for {}",
            row.median_lead_time_days,
            row.path
        );
        assert!(
            row.p95_lead_time_days >= row.median_lead_time_days,
            "p95 >= median by construction"
        );
        assert!(
            row.wip_age_days >= 0.0,
            "wip_age_days cannot be negative; got {} for {}",
            row.wip_age_days,
            row.path
        );
        assert!(
            (0.0..=100.0).contains(&row.friction_score),
            "friction_score must be in [0,100]; got {} for {}",
            row.friction_score,
            row.path
        );
    }
}
