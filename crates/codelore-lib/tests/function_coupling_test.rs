/// Integration tests for the `function-coupling` analysis.
///
/// Uses `function_xray_repo` which after meta-tweak-1..10 has `hot` and
/// `cold` co-changing in 3 revisions. The coupling test asserts `p_value` is
/// `Some(p) < 0.1` (non-degenerate Fisher table) and `confidence = 1.0`.
///
/// Expected values (n = 17, from distinct revs in hunks for `src/target.rs`;
/// seed is an Add → no hunks; tweak-1/2/3 + tweak-mh + coupled-1/2/3 +
/// meta-tweak-1..10 = 17 revs):
///
/// ```text
/// hot_changes  = 7   (tweak-1/2/3, tweak-mh, coupled-1/2/3)
/// cold_changes = 3   (coupled-1/2/3 only)
/// co_changes   = 3   (coupled-1/2/3 touch both)
/// a_only       = 4   (hot_changes - co = 7 - 3)
/// b_only       = 0   (cold_changes - co = 3 - 3)
/// neither      = 10  (n - co - a_only - b_only = 17 - 3 - 4 - 0;
///                     meta-tweak-1..10 touch neither hot nor cold)
/// Fisher table [3, 4, 0, 10]: row1=7, row2=10, col1=3, col2=14, N=17
///   P(k=3) = C(7,3)·C(10,0)/C(17,3) = 35/680 ≈ 0.051 < 0.1
///   Two-tail sums only k=3 (only table with P ≤ P_observed ≈ 0.051)
///   → p ≈ 0.051 < 0.1
/// confidence   = 1.0  (co / min(hot_changes, cold_changes) = 3/3)
/// ```
///
/// Note: `None`-sorts-first is still the correct behavior for genuinely
/// degenerate tables on real repos (zero row or column sum → p → 0 in limit).
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
    /// Confidence must be 1.0 (co/min(a,b) = 3/3). `p_value` must be
    /// `Some(p) < 0.1` — the Fisher table [3, 4, 0, 10] is non-degenerate
    /// because meta-tweak-1..10 contribute `neither = 10`, giving p ≈ 0.051.
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
        // Fisher table [co=3, a_only=4, b_only=0, neither=10].
        // neither=10 comes from meta-tweak-1..10 each touching only `meta`.
        // Row2 = b_only + neither = 0 + 10 = 10 → non-degenerate → Some(p).
        // P(k=3) = C(7,3)·C(10,0)/C(17,3) = 35/680 ≈ 0.051 < 0.1.
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
