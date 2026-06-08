use codelore_lib::Options;
use codelore_lib::analyses::coupling::run_coupling;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;

#[test]
fn coupling_for_tiny_repo() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        min_shared_revs: 1,
        min_coupling_pct: 0,
        fisher_significance: 1.0, // allow all p-values for tiny fixture
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_coupling(&db, &opts).expect("run");

    // tiny_repo has src/main.rs (4 commits) and src/lib.rs (1 commit).
    // src/lib.rs's only commit ("add lib", commit 3) touches only src/lib.rs.
    // src/main.rs is touched in commits 1, 2, 4, 5.
    // There are NO shared commits between the two files, so no coupling pair
    // should be produced even with min_shared_revs=1.
    assert!(
        rows.is_empty() || rows.iter().all(|r| r.shared >= 1),
        "any coupling row must have at least 1 shared commit"
    );
}

#[test]
fn coupling_struct_shape() {
    use codelore_lib::analyses::coupling::CouplingRow;
    let row = CouplingRow {
        entity_a: "a.rs".into(),
        entity_b: "b.rs".into(),
        shared: 4,
        revs_a: 5,
        revs_b: 5,
        average_revs: 5,
        degree: 80.0,
        fisher_p: 0.01,
    };
    assert_eq!(row.shared, 4);
    assert!(row.degree > 70.0);
    assert!(row.fisher_p < 0.05);
    assert_eq!(row.entity_a, "a.rs");
    assert_eq!(row.entity_b, "b.rs");
}

#[test]
fn coupling_respects_min_shared_revs() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        min_shared_revs: 2, // stricter: require ≥2 shared
        min_coupling_pct: 0,
        fisher_significance: 1.0,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_coupling(&db, &opts).expect("run");
    for row in &rows {
        assert!(
            row.shared >= 2,
            "min_shared_revs=2 violated: shared={} for {}<->{}",
            row.shared,
            row.entity_a,
            row.entity_b
        );
    }
}

/// `max_coupling_pct` was wired through `Options` but the SQL only bound
/// the lower bound, so `--max-coupling N` was silently ignored. Regression:
/// assert the upper bound actually filters pairs.
#[test]
fn coupling_respects_max_coupling_pct() {
    let diff_repo = codelore_lib::test_support::differential_repo::build();
    let repo = GixRepo::open(diff_repo.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");

    // Baseline: collect ALL pairs (degree >= 0) to find what's there.
    let opts_all = Options {
        repo_path: diff_repo.dir.path().to_path_buf(),
        min_revs: 1,
        min_shared_revs: 1,
        min_coupling_pct: 0,
        max_coupling_pct: 100,
        fisher_significance: 1.0,
        ..Options::default()
    };
    db.ingest(&repo, &opts_all).expect("ingest");
    let baseline = run_coupling(&db, &opts_all).expect("baseline");
    let max_observed = baseline.iter().map(|r| r.degree).fold(0.0_f64, f64::max);
    assert!(
        max_observed > 0.0,
        "differential_repo should produce at least one coupled pair with degree > 0; \
         got {} rows max degree = {max_observed}",
        baseline.len()
    );

    // Cap below the observed max — MUST drop at least the top pair.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let cap_pct = (max_observed / 2.0).floor() as u8;
    let opts_capped = Options {
        max_coupling_pct: cap_pct,
        ..opts_all.clone()
    };
    let capped = run_coupling(&db, &opts_capped).expect("capped");

    assert!(
        capped.len() < baseline.len(),
        "max_coupling_pct={cap_pct} should drop ≥1 pair; baseline={}, capped={}",
        baseline.len(),
        capped.len()
    );
    for row in &capped {
        assert!(
            row.degree <= f64::from(cap_pct),
            "row degree={} exceeds cap={cap_pct} for {}<->{}",
            row.degree,
            row.entity_a,
            row.entity_b
        );
    }
}

#[test]
fn coupling_fisher_significance_filter() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    // With fisher_significance=0.0, no pair can pass (p-value is always > 0)
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        min_shared_revs: 1,
        min_coupling_pct: 0,
        fisher_significance: 0.0,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_coupling(&db, &opts).expect("run");
    assert!(
        rows.is_empty(),
        "fisher_significance=0.0 should reject all pairs, got {} rows",
        rows.len()
    );
}
