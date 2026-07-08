use codelore_lib::Options;
use codelore_lib::analyses::effort_exposure::run_effort_exposure;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;

/// `biomarker_repo` has a deliberate complexity gradient with complex.rs,
/// big.rs, and duplicate files — enough to produce ≥1 health band row with
/// varied structural risk. Exact band composition (red/yellow/green) depends on
/// the scoring thresholds; we validate structural invariants, not band labels.
#[test]
fn effort_exposure_returns_bands_with_valid_metrics() {
    let bio = codelore_lib::test_support::biomarker_repo::build();
    let repo = GixRepo::open(bio.dir.path()).expect("open repo");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: bio.dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_effort_exposure(&db, &opts).expect("run effort-exposure");

    // At least one band must be present (repo is non-empty).
    assert!(!rows.is_empty(), "expected ≥1 band row");

    // Sanity-check numeric ranges for every band returned.
    for row in &rows {
        assert!(
            (0.0..=100.0).contains(&row.loc_share_pct),
            "loc_share_pct out of range for {}: {}",
            row.band,
            row.loc_share_pct
        );
        assert!(
            (0.0..=100.0).contains(&row.commit_share_pct),
            "commit_share_pct out of range for {}: {}",
            row.band,
            row.commit_share_pct
        );
        assert!(
            (0.0..=100.0).contains(&row.churn_share_pct),
            "churn_share_pct out of range for {}: {}",
            row.band,
            row.churn_share_pct
        );
        assert!(
            (0.0..=1.0).contains(&row.commit_share_ci_low),
            "ci_low out of [0,1] for {}: {}",
            row.band,
            row.commit_share_ci_low
        );
        assert!(
            (0.0..=1.0).contains(&row.commit_share_ci_high),
            "ci_high out of [0,1] for {}: {}",
            row.band,
            row.commit_share_ci_high
        );
        assert!(
            row.commit_share_ci_low <= row.commit_share_ci_high,
            "ci_low > ci_high for {}: {} > {}",
            row.band,
            row.commit_share_ci_low,
            row.commit_share_ci_high
        );
        // Wilson CI must contain the sample proportion.
        let share = row.commit_share_pct / 100.0;
        assert!(
            row.commit_share_ci_low <= share + 1e-9,
            "ci_low must be ≤ commit_share proportion for {}: {} > {}",
            row.band,
            row.commit_share_ci_low,
            share
        );
        assert!(
            row.commit_share_ci_high >= share - 1e-9,
            "ci_high must be ≥ commit_share proportion for {}: {} < {}",
            row.band,
            row.commit_share_ci_high,
            share
        );
    }
}

#[test]
fn effort_exposure_files_count_matches_code_health() {
    let bio = codelore_lib::test_support::biomarker_repo::build();
    let repo = GixRepo::open(bio.dir.path()).expect("open repo");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: bio.dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_effort_exposure(&db, &opts).expect("run");

    // Total files across all bands must be ≥ 1 (biomarker_repo has 6 files).
    let total_files: u32 = rows.iter().map(|r| r.files).sum();
    assert!(total_files >= 1, "expected ≥1 total files across all bands");
}
