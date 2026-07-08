/// Integration tests for the `function-coupling` analysis.
///
/// Uses `function_xray_repo` which after coupled-1/2/3 commits has `hot` and
/// `cold` co-changing in 3 revisions. The coupling test asserts `p_value`
/// is `None` (degenerate-marginal = perfectly coupled) and confidence = 1.0.
///
/// Expected values (n = 7 non-seed revisions in the hunks table):
///
/// ```text
/// hot_changes  = 7  (tweak-1/2/3, tweak-mh, coupled-1/2/3)
/// cold_changes = 3  (coupled-1/2/3 only)
/// co_changes   = 3  (coupled-1/2/3 touch both)
/// a_only       = 4  (hot_changes - co)
/// b_only       = 0  (cold_changes - co)
/// neither      = 0  (n - co - a_only - b_only = 7 - 3 - 4 - 0)
/// Fisher returns None for [3, 4, 0, 0]: row2 = c+d = 0 is a degenerate
/// marginal; None means perfectly coupled (p = 0), sorts before any Some.
/// confidence   = 1.0  (co / min(hot_changes, cold_changes) = 3/3)
/// ```
#[cfg(feature = "test-support")]
mod function_coupling_integration {
    use codelore_lib::Options;
    use codelore_lib::analyses::function_coupling::run_function_coupling;
    use codelore_lib::facts::FactsDb;
    use codelore_lib::repo::GixRepo;
    use codelore_lib::test_support::function_xray_repo;

    fn build_db_and_rows(
        target: &str,
    ) -> (
        function_xray_repo::FunctionXrayRepo,
        Vec<codelore_lib::analyses::function_coupling::FunctionCouplingRow>,
    ) {
        let repo = function_xray_repo::build();
        let gix = GixRepo::open(repo.dir.path()).expect("open repo");
        let opts = Options {
            repo_path: repo.dir.path().to_path_buf(),
            min_revs: 1,
            ..Options::default()
        };
        let db = FactsDb::new_in_memory().expect("new_in_memory");
        db.ingest(&gix, &opts).expect("ingest");
        let rows = run_function_coupling(&db, &gix, &opts, target).expect("run_function_coupling");
        (repo, rows)
    }

    /// `hot` and `cold` co-change in 3 revisions (coupled-1/2/3).
    /// Confidence must be 1.0 (co/min(a,b) = 3/3). `p_value` is `None`
    /// because the Fisher table [3,4,0,0] has a zero marginal (row2 = 0),
    /// which the implementation treats as a degenerate case that sorts first.
    #[test]
    fn hot_cold_pair_has_confidence_1_and_low_p() {
        let (_repo, rows) = build_db_and_rows("src/target.rs");

        let pair = rows
            .iter()
            .find(|r| {
                (r.a.starts_with("hot@") && r.b.starts_with("cold@"))
                    || (r.a.starts_with("cold@") && r.b.starts_with("hot@"))
            })
            .expect("expected a (hot, cold) coupling row");

        assert_eq!(
            pair.co_changes, 3,
            "expected co_changes = 3 (coupled-1/2/3), got {}",
            pair.co_changes
        );
        assert!(
            (pair.confidence - 1.0).abs() < 1e-9,
            "expected confidence = 1.0 (co/min(a,b)=3/3), got {}",
            pair.confidence
        );
        // Fisher table [co=3, a_only=4, b_only=0, neither=0] has row2=c+d=0,
        // a degenerate marginal (p → 0). The implementation returns None and
        // sorts None first as the strongest coupling signal.
        assert!(
            pair.p_value.is_none(),
            "expected p_value = None (degenerate marginal, perfectly coupled), \
             got {:?}",
            pair.p_value
        );
    }

    /// Non-existent target returns empty, not an error.
    #[test]
    fn nonexistent_target_returns_empty() {
        let (_repo, rows) = build_db_and_rows("src/does_not_exist.rs");
        assert!(rows.is_empty(), "expected empty for nonexistent path");
    }
}
