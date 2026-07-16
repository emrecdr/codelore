//! Integration tests for the per-file enrichment fact sheet: determinism
//! across two builds on the same fact store, the mandatory-code-health error
//! path, and numeric-value extraction. Uses the biomarker fixture (a repo with
//! a deliberate complexity gradient and co-changed files) ingested through a
//! real `FactsDb`, mirroring the ingest pattern in `defect_calibration_test`.

use codelore_lib::CodeLoreError;
use codelore_lib::analyses::code_health::run_code_health;
use codelore_lib::enrichment::fact_sheet::FileFactSheet;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;
use codelore_lib::test_support::{biomarker_repo, permissive_coupling_opts};

/// Ingest the biomarker fixture and return the pieces `FileFactSheet::build`
/// needs. The `TempDir` inside `BiomarkerRepo` must outlive the returned repo,
/// so the fixture is returned too.
fn ingest_biomarker_fixture() -> (
    biomarker_repo::BiomarkerRepo,
    GixRepo,
    FactsDb,
    codelore_lib::Options,
) {
    let fx = biomarker_repo::build();
    let repo = GixRepo::open(fx.dir.path()).expect("open fixture");
    let db = FactsDb::new_in_memory().expect("in-memory db");
    let opts = permissive_coupling_opts(fx.dir.path().to_path_buf());
    db.ingest(&repo, &opts).expect("ingest fixture");
    (fx, repo, db, opts)
}

#[test]
fn fact_sheet_is_deterministic() {
    let (_fx, repo, db, opts) = ingest_biomarker_fixture();

    // Pick a real tracked path (the worst-scoring file is first, deterministic)
    // and remember its band so we can assert the canonical text carries it.
    let health = run_code_health(&db, &opts.with_no_row_limit()).expect("code-health");
    let target = health
        .first()
        .expect("fixture yields code-health rows")
        .path
        .clone();
    let band = health[0].band.clone();

    let first = FileFactSheet::build(&db, &repo, &opts, &target).expect("build 1");
    let second = FileFactSheet::build(&db, &repo, &opts, &target).expect("build 2");

    assert_eq!(
        first.to_canonical_text(),
        second.to_canonical_text(),
        "two builds over the same fact store must be byte-identical"
    );
    assert_eq!(
        first.digest(),
        second.digest(),
        "equal text -> equal digest"
    );

    let canonical = first.to_canonical_text();
    assert!(
        canonical.contains("code-health"),
        "canonical text must carry the mandatory code-health section: {canonical}"
    );
    assert!(
        canonical.contains(&band),
        "canonical text must carry the file's band ({band}): {canonical}"
    );
}

#[test]
fn fact_sheet_unknown_path_errors() {
    let (_fx, repo, db, opts) = ingest_biomarker_fixture();

    let err = FileFactSheet::build(&db, &repo, &opts, "src/does_not_exist.rs")
        .expect_err("an untracked path has no code-health data");
    assert!(
        matches!(err, CodeLoreError::Analysis(_)),
        "a missing code-health row is an analysis error, got: {err:?}"
    );
    assert!(
        err.to_string().contains("src/does_not_exist.rs"),
        "the error message must name the offending path: {err}"
    );
}

#[test]
fn numeric_values_extracts_floats() {
    // A hand-built sheet: only whole-number-parseable values are extracted, in
    // section then key order; string values (band, author) are skipped.
    let sheet = FileFactSheet {
        path: "src/foo.rs".to_string(),
        sections: vec![
            (
                "code-health".to_string(),
                vec![
                    ("score".to_string(), "72.5".to_string()),
                    ("band".to_string(), "yellow".to_string()),
                    ("structural_risk".to_string(), "0.41".to_string()),
                ],
            ),
            (
                "ownership".to_string(),
                vec![
                    ("main_author".to_string(), "Bio".to_string()),
                    ("total_revs".to_string(), "6".to_string()),
                ],
            ),
        ],
    };
    assert_eq!(sheet.numeric_values(), vec![72.5, 0.41, 6.0]);
}
