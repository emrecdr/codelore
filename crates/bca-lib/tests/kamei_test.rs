use bca_lib::Options;
use bca_lib::facts::FactsDb;
use bca_lib::repo::GixRepo;

#[test]
fn kamei_features_populated_for_tiny_repo() {
    let tiny = bca_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");

    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    // NF should be >= 1 for every commit (each commit touches at least one file)
    let min_nf: String = db
        .query_one_value("SELECT CAST(MIN(nf) AS TEXT) FROM commits WHERE nf IS NOT NULL")
        .expect("nf query");
    assert!(min_nf.parse::<u32>().unwrap() >= 1);

    // NS should be >= 1 — all files in tiny_repo live under "src/"
    let min_ns_val: String = db
        .query_one_value("SELECT CAST(MIN(ns) AS TEXT) FROM commits WHERE ns IS NOT NULL")
        .expect("ns query");
    assert!(
        min_ns_val.parse::<u32>().unwrap() >= 1,
        "expected NS >= 1 (files in src/)"
    );

    // LA column should be populated (not NULL) for all commits.
    // Plan 4 stubs loc_added=0 at gix walk time (blob-diff lands in Plan 5),
    // so SUM(la) = 0 is expected — we verify it is at least non-NULL.
    let la_non_null: String = db
        .query_one_value("SELECT CAST(COUNT(*) AS TEXT) FROM commits WHERE la IS NOT NULL")
        .expect("la non-null query");
    assert_eq!(
        la_non_null.parse::<u32>().unwrap(),
        5,
        "expected all 5 commits to have la populated (even if 0)"
    );

    // EXP should be 0 for the first commit, max = 4 for the 5th commit (same author)
    let max_exp: String = db
        .query_one_value("SELECT CAST(MAX(exp) AS TEXT) FROM commits")
        .expect("exp query");
    let max_exp_val: u32 = max_exp.parse().unwrap();
    assert!(
        max_exp_val >= 4,
        "expected max EXP >= 4 for 5-commit single-author repo, got {max_exp_val}"
    );
}

#[test]
fn kamei_fix_flag_detects_bug_keywords() {
    // Build a fixture where one commit message says "fix typo"
    // tiny_repo's commit 4 has message "fix typo" — should set fix=TRUE
    let tiny = bca_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let fix_count: String = db
        .query_one_value("SELECT CAST(COUNT(*) AS TEXT) FROM commits WHERE fix = TRUE")
        .expect("fix count");
    assert!(
        fix_count.parse::<u32>().unwrap() >= 1,
        "tiny repo has 'fix typo' commit; expected >=1 FIX"
    );
}
