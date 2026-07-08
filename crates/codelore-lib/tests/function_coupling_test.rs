/// Integration tests for the `function-coupling` analysis.
///
/// Uses `function_xray_repo` which has `hot`, `cold`, and `meta` functions.
/// After meta-tweak, the Fisher table for (hot, cold) is non-degenerate:
///
/// ```text
/// n            = 8  (total revisions in hunks table — seed not counted)
/// hot_changes  = 7  (tweak-1/2/3, tweak-mh, coupled-1/2/3)
/// cold_changes = 3  (coupled-1/2/3 only)
/// co_changes   = 3  (coupled-1/2/3 touch both hot and cold)
/// a_only       = 4  (hot_changes - co = 7 - 3)
/// b_only       = 0  (cold_changes - co = 3 - 3)
/// neither      = 1  (n - co - a_only - b_only = 8 - 3 - 4 - 0)
///                   ← meta-tweak touches only `meta`, leaving both untouched
/// confidence   = 1.0  (co / min(hot_changes, cold_changes) = 3/3)
/// Fisher p for [3, 4, 0, 1]: row1=7, row2=1, col1=3, col2=5 — computable,
///   strongly significant (p < 0.05) given the high co-change rate.
/// ```
///
/// Note: the `None`-sorts-first behavior in `run_function_coupling` is still
/// correct for real degenerate repos where a marginal is truly zero. The
/// fixture now avoids the degenerate case to give a concrete p-value in tests.
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
    /// Confidence must be 1.0 (co/min(a,b) = 3/3). `p_value` must be `Some`
    /// and < 0.1 — the Fisher table [3, 4, 0, 1] is non-degenerate because
    /// meta-tweak contributes `neither = 1`.
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
        // Fisher table [co=3, a_only=4, b_only=0, neither=1].
        // neither=1 comes from meta-tweak touching only `meta`.
        // Row2 = b_only + neither = 0 + 1 = 1 → non-degenerate → Some(p).
        let p = pair
            .p_value
            .expect("p_value must be Some; Fisher table [3,4,0,1] is non-degenerate");
        assert!(
            p < 0.1,
            "expected p_value < 0.1 for a strongly coupled pair, got {p}"
        );
    }

    /// Non-existent target returns empty, not an error.
    #[test]
    fn nonexistent_target_returns_empty() {
        let (_repo, rows) = build_db_and_rows("src/does_not_exist.rs");
        assert!(rows.is_empty(), "expected empty for nonexistent path");
    }
}
