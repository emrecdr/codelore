use codelore_lib::Options;
use codelore_lib::analyses::code_health::run_code_health;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;

#[test]
fn code_health_for_tiny_repo() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_code_health(&db, &opts).expect("run");
    assert!(!rows.is_empty(), "should produce ≥1 row");

    for row in &rows {
        assert!(
            row.score >= 0.0 && row.score <= 100.0,
            "score should be in [0, 100], got {} for {}",
            row.score,
            row.path
        );
        assert!(
            row.cognitive >= 0.0,
            "cognitive should be >= 0, got {} for {}",
            row.cognitive,
            row.path
        );
    }
}

#[test]
fn code_health_ranks_least_healthy_first() {
    // Convention: ORDER BY score ASC — least healthy first (these are the
    // ones a developer should look at).
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_code_health(&db, &opts).expect("run");
    // Confirm ascending order
    for w in rows.windows(2) {
        assert!(
            w[0].score <= w[1].score,
            "expected ascending score order, got {} > {}",
            w[0].score,
            w[1].score
        );
    }
}

#[test]
fn code_health_penalizes_churn() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_code_health(&db, &opts).expect("run");
    // tiny_repo has src/main.rs (4 commits = high churn) and src/lib.rs (1 commit = low churn).
    // src/main.rs should rank LOWER (less healthy) than src/lib.rs in Code Health.
    let main = rows.iter().find(|r| r.path == "src/main.rs");
    let lib = rows.iter().find(|r| r.path == "src/lib.rs");
    if let (Some(m), Some(l)) = (main, lib) {
        assert!(
            m.score <= l.score,
            "src/main.rs (4 commits) should rank <= src/lib.rs (1 commit) in code health, got main={} lib={}",
            m.score,
            l.score
        );
    }
}

#[test]
fn code_health_reports_band_and_percentile() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = codelore_lib::repo::GixRepo::open(tiny.dir.path()).expect("open");
    let db = codelore_lib::facts::FactsDb::new_in_memory().expect("db");
    let opts = codelore_lib::Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        ..codelore_lib::Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = codelore_lib::analyses::code_health::run_code_health(&db, &opts).expect("run");
    assert!(!rows.is_empty());
    for row in &rows {
        assert!(
            (0.0..=1.0).contains(&row.percentile),
            "percentile in [0,1]: {}",
            row.percentile
        );
        assert!(
            matches!(row.band.as_str(), "red" | "yellow" | "green"),
            "band must be red|yellow|green, got {}",
            row.band
        );
        assert!((0.0..=1.0).contains(&row.structural_risk), "structural_risk in [0,1]: {}", row.structural_risk);
    }
}

#[test]
fn biomarkers_flag_complex_functions() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = codelore_lib::repo::GixRepo::open(tiny.dir.path()).expect("open");
    let db = codelore_lib::facts::FactsDb::new_in_memory().expect("db");
    let opts = codelore_lib::Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        ..codelore_lib::Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    // Running code-health materializes the biomarker table as a side effect.
    let _ = codelore_lib::analyses::code_health::run_code_health(&db, &opts).expect("run");

    let count: i64 = db
        .query_row("SELECT COUNT(*) FROM code_health_biomarkers_v1", [], |r| r.get(0))
        .expect("query biomarkers");
    assert!(count >= 1, "tiny_repo should produce >=1 biomarker row");

    // intensities are valid probabilities
    let bad: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM code_health_biomarkers_v1 WHERE intensity < 0.0 OR intensity > 1.0",
            [], |r| r.get(0),
        )
        .expect("query range");
    assert_eq!(bad, 0, "all intensities must be in [0,1]");
}

/// `--rows N` MUST NOT change the score computed for a path that survives
/// the truncation. The bug it regression-protects: `materialize_centrality`
/// used to pass the parent `opts` (with `rows_limit = N`) straight into
/// `run_coupling`, so the centrality term was computed over a sliver of
/// the coupling graph and the final score drifted by `rows_limit`.
#[test]
fn code_health_score_invariant_under_rows_limit() {
    let diff_repo = codelore_lib::test_support::differential_repo::build();
    let repo = GixRepo::open(diff_repo.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts_unlimited = Options {
        repo_path: diff_repo.dir.path().to_path_buf(),
        min_revs: 1,
        fisher_significance: 1.0,
        rows_limit: None,
        ..Options::default()
    };
    db.ingest(&repo, &opts_unlimited).expect("ingest");
    let baseline = run_code_health(&db, &opts_unlimited).expect("baseline");
    assert!(baseline.len() >= 2, "need ≥2 rows to test truncation");

    let opts_capped = Options {
        rows_limit: Some(2),
        ..opts_unlimited.clone()
    };
    let capped = run_code_health(&db, &opts_capped).expect("capped");
    assert!(capped.len() <= 2, "rows_limit=2 should truncate output");

    // Each capped row's score MUST match the baseline score for the same path.
    // If the centrality term were computed over a truncated coupling graph,
    // these would drift.
    for row in &capped {
        let baseline_row = baseline
            .iter()
            .find(|b| b.path == row.path)
            .expect("capped path must be in baseline");
        assert!(
            (row.score - baseline_row.score).abs() < 1e-9,
            "score drift for {}: capped={} baseline={} — rows_limit leaked into centrality?",
            row.path,
            row.score,
            baseline_row.score
        );
    }
}
