/// Integration tests for the `function-xray` analysis.
///
/// Uses `function_xray_repo`, a purpose-built fixture with two functions in
/// `src/target.rs`:
///   - `hot` — 8-line function; body modified in tweak-1/2/3, tweak-mh, and
///     coupled-1/2/3 → `change_freq` = 7 (all non-seed commits touch `hot`)
///   - `cold` — touched only in coupled-1/2/3 → `change_freq` = 3
///
/// The multi-hunk commit (tweak-mh) edits two regions of `hot` that are 6
/// lines apart, producing two separate hunks in one revision. The dedup
/// assertion verifies that `change_freq` increments by exactly 1 for that
/// commit, not 2.
///
/// The hunk-overlap unit tests (`analyses::function_xray::tests::overlap_predicate`)
/// cover the predicate in isolation across all edge cases including pure deletions.
#[cfg(feature = "test-support")]
mod function_xray_integration {
    use codelore_lib::Options;
    use codelore_lib::analyses::function_xray::run_function_xray;
    use codelore_lib::facts::FactsDb;
    use codelore_lib::repo::GixRepo;
    use codelore_lib::test_support::function_xray_repo;

    fn build_db_and_rows(
        target: &str,
    ) -> (
        function_xray_repo::FunctionXrayRepo,
        Vec<codelore_lib::analyses::function_xray::FunctionXrayRow>,
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
        let rows = run_function_xray(&db, &gix, &opts, target).expect("run_function_xray");
        (repo, rows)
    }

    /// `hot` must have the highest `change_freq`; `cold` must have lower freq.
    ///
    /// The deduped name format is `{fn_name}@{start_line}-{end_line}` (see
    /// `facts::ingest::consumer::dedup_entities`). `hot` spans lines 1-9;
    /// `cold` follows a blank separator at line 10, spanning lines 11-13.
    ///
    /// 7 revisions touch `hot`: tweak-1/2/3 (single-hunk), tweak-mh (two hunks
    /// counted once), coupled-1/2/3 (each touches hot + cold).
    /// 3 revisions touch `cold`: coupled-1/2/3 only.
    #[test]
    fn hot_is_hottest_function() {
        let (_repo, rows) = build_db_and_rows("src/target.rs");

        assert!(
            !rows.is_empty(),
            "expected rows for src/target.rs; got empty"
        );

        // Top row must be `hot` (highest change_freq = 7).
        let hot = &rows[0];
        assert!(
            hot.function.starts_with("hot@"),
            "expected top function to start with 'hot@', got '{}'",
            hot.function
        );
        assert_eq!(
            hot.change_freq, 7,
            "expected hot change_freq = 7 (tweak-1/2/3 + tweak-mh + coupled-1/2/3), got {}",
            hot.change_freq
        );

        // `cold` must be present and have change_freq = 3 (coupled-1/2/3 only).
        let cold = rows
            .iter()
            .find(|r| r.function.starts_with("cold@"))
            .expect("expected a 'cold@...' row");
        assert_eq!(
            cold.change_freq, 3,
            "expected cold change_freq = 3 (coupled-1/2/3), got {}",
            cold.change_freq
        );

        // Sanity: hot must have loc > 0 (alive at HEAD).
        assert!(hot.loc > 0, "expected hot.loc > 0, got {}", hot.loc);

        // last_changed must be non-empty for hot (it was changed).
        assert!(
            !hot.last_changed.is_empty(),
            "expected hot.last_changed to be non-empty"
        );
    }

    /// The multi-hunk commit (tweak-mh) edits two separated regions of `hot`
    /// in a single revision. `change_freq` must count that revision once, not
    /// once per hunk.
    ///
    /// This is the direct regression guard for the rev-dedup bug: without
    /// deduplication on `(function, rev)`, the two hunks in tweak-mh would
    /// each increment `change_freq`. With 7 total revisions for `hot`, a
    /// double-count from tweak-mh would produce 8.
    #[test]
    fn multi_hunk_commit_counts_as_one_revision() {
        let (_repo, rows) = build_db_and_rows("src/target.rs");

        let hot = rows
            .iter()
            .find(|r| r.function.starts_with("hot@"))
            .expect("expected a 'hot@...' row");

        // 3 single-hunk + 1 multi-hunk + 3 coupled = 7 distinct revisions.
        // Without (function, rev) dedup the multi-hunk commit would add 2,
        // yielding 8.
        assert_eq!(
            hot.change_freq, 7,
            "multi-hunk commit must count as 1 revision, not 2; got change_freq = {}",
            hot.change_freq
        );
    }

    /// Non-existent target path returns empty, not an error.
    #[test]
    fn nonexistent_target_returns_empty() {
        let (_repo, rows) = build_db_and_rows("src/does_not_exist.rs");
        assert!(
            rows.is_empty(),
            "expected empty rows for a path that never existed"
        );
    }
}
