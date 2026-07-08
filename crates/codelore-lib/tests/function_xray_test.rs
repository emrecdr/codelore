/// Integration tests for the `function-xray` analysis.
///
/// Uses `function_xray_repo`, a purpose-built fixture with two functions in
/// `src/target.rs`:
///   - `hot` — body modified in 3 commits after seed → change_freq = 3
///   - `cold` — never changed after seed → change_freq = 0
///
/// Function span is 3 lines (lines 1–3): the hunk always lands at
/// new_start = 2, new_lines = 1, which overlaps [1, 3] correctly.
///
/// The hunk-overlap unit tests (`analyses::function_xray::tests::overlap_predicate`)
/// cover the predicate in isolation across all edge cases including pure deletions.
#[cfg(feature = "test-support")]
mod function_xray_integration {
    use codelore_lib::analyses::function_xray::run_function_xray;
    use codelore_lib::facts::FactsDb;
    use codelore_lib::repo::GixRepo;
    use codelore_lib::test_support::function_xray_repo;
    use codelore_lib::Options;

    /// `hot@1-3` must have change_freq = 3; `cold@5-7` must have change_freq = 0.
    ///
    /// The deduped name format is `{fn_name}@{start_line}-{end_line}` (see
    /// `facts::ingest::consumer::dedup_entities`). `hot` spans lines 1–3;
    /// `cold` follows a blank separator at line 4, spanning lines 5–7.
    #[test]
    fn hot_has_freq_3_cold_has_freq_0() {
        let repo = function_xray_repo::build();
        let gix = GixRepo::open(repo.dir.path()).expect("open repo");

        let opts = Options {
            repo_path: repo.dir.path().to_path_buf(),
            min_revs: 1,
            ..Options::default()
        };

        let db = FactsDb::new_in_memory().expect("new_in_memory");
        db.ingest(&gix, &opts).expect("ingest");

        let rows = run_function_xray(&db, &gix, &opts, "src/target.rs")
            .expect("run_function_xray");

        assert!(
            !rows.is_empty(),
            "expected rows for src/target.rs; got empty"
        );

        // Top row must be `hot` (highest change_freq = 3).
        let hot = &rows[0];
        assert!(
            hot.function.starts_with("hot@"),
            "expected top function to start with 'hot@', got '{}'",
            hot.function
        );
        assert_eq!(
            hot.change_freq, 3,
            "expected hot change_freq = 3, got {}",
            hot.change_freq
        );

        // `cold` must be present and have change_freq = 0.
        let cold = rows
            .iter()
            .find(|r| r.function.starts_with("cold@"))
            .expect("expected a 'cold@...' row");
        assert_eq!(
            cold.change_freq, 0,
            "expected cold change_freq = 0, got {}",
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

    /// Non-existent target path returns empty — not an error.
    #[test]
    fn nonexistent_target_returns_empty() {
        let repo = function_xray_repo::build();
        let gix = GixRepo::open(repo.dir.path()).expect("open repo");

        let opts = Options {
            repo_path: repo.dir.path().to_path_buf(),
            min_revs: 1,
            ..Options::default()
        };

        let db = FactsDb::new_in_memory().expect("new_in_memory");
        db.ingest(&gix, &opts).expect("ingest");

        let rows = run_function_xray(&db, &gix, &opts, "src/does_not_exist.rs")
            .expect("run_function_xray on nonexistent path must not error");

        assert!(
            rows.is_empty(),
            "expected empty rows for a path that never existed"
        );
    }
}
