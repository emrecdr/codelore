/// Integration tests for the `function-hotspots` analysis.
///
/// Uses `function_hotspots_repo`, a purpose-built fixture with two functions
/// in `src/target.rs`:
///   - `hot` — a 7-line function with a real `if`/`else` branch, edited on
///     its branch return value in 6 separate commits (`hot-tweak-1..6`) →
///     `revs` = 6.
///   - `stable` — a 3-line function with no branching, edited on its body
///     in 2 separate commits (`stable-tweak-1..2`) → `revs` = 2.
///
/// `hot` carries real cognitive complexity (the branch); `stable` does not.
/// This is deliberate — see the fixture's module doc for why a
/// flat-complexity (all-zero) fixture would collapse `function_hotspot_score`
/// to a `(path, function)` tiebreak instead of a real score gap.
///
/// The hunk-overlap predicate itself (transliterated from
/// `function_xray::hunk_overlaps` into SQL) is unit-tested in isolation by
/// `analyses::function_xray::tests::overlap_predicate` — this test exercises
/// the repo-wide SQL join end to end against real hunk data.
#[cfg(feature = "test-support")]
mod function_hotspots_integration {
    use codelore_lib::Options;
    use codelore_lib::analyses::function_hotspots::run_function_hotspots;
    use codelore_lib::facts::FactsDb;
    use codelore_lib::repo::GixRepo;
    use codelore_lib::test_support::function_hotspots_repo;

    fn build_db_and_rows(
        min_revs: u32,
    ) -> Vec<codelore_lib::analyses::function_hotspots::FunctionHotspotRow> {
        let repo = function_hotspots_repo::build();
        let gix = GixRepo::open(repo.dir.path()).expect("open repo");
        let opts = Options {
            repo_path: repo.dir.path().to_path_buf(),
            min_revs,
            ..Options::default()
        };
        let db = FactsDb::new_in_memory().expect("new_in_memory");
        db.ingest(&gix, &opts).expect("ingest");
        run_function_hotspots(&db, &opts).expect("run_function_hotspots")
    }

    /// At `--min-revs 1` both functions clear the floor. `hot`'s `revs` must
    /// match its exact churn count (6), `stable`'s must match its own (2),
    /// and `hot` — carrying both more revisions AND non-zero cognitive
    /// complexity — must rank strictly above `stable` on
    /// `function_hotspot_score` (not merely via a tiebreak: `stable`'s score
    /// is exactly 0.0 because its cognitive complexity is 0, so
    /// `cognitive_health` bottoms out at 100 and `(100 - cognitive_health)`
    /// zeroes the whole product).
    #[test]
    fn hot_outranks_stable_and_revs_match_churn_count() {
        let rows = build_db_and_rows(1);
        assert!(!rows.is_empty(), "expected rows at --min-revs 1; got empty");

        let hot = rows
            .iter()
            .find(|r| r.function.starts_with("hot@"))
            .expect("expected a 'hot@...' row");
        let stable = rows
            .iter()
            .find(|r| r.function.starts_with("stable@"))
            .expect("expected a 'stable@...' row");

        assert_eq!(hot.revs, 6, "hot revs must match its 6 tweak commits");
        assert_eq!(stable.revs, 2, "stable revs must match its 2 tweak commits");

        assert!(
            hot.cognitive > 0.0,
            "hot has a real if/else branch, expected cognitive > 0, got {}",
            hot.cognitive
        );
        assert!(
            (stable.cognitive).abs() < f64::EPSILON,
            "stable has no branching, expected cognitive == 0, got {}",
            stable.cognitive
        );

        assert!(
            hot.function_hotspot_score > stable.function_hotspot_score,
            "hot ({}) must outrank stable ({}) on function_hotspot_score",
            hot.function_hotspot_score,
            stable.function_hotspot_score
        );
        assert!(
            (stable.function_hotspot_score).abs() < 1e-9,
            "stable's score must be ~0 (zero cognitive complexity), got {}",
            stable.function_hotspot_score
        );

        // hot must be the top-ranked row by score, not merely present.
        assert!(
            rows[0].function.starts_with("hot@"),
            "expected 'hot@...' to rank first, got '{}'",
            rows[0].function
        );
    }

    /// Raising `--min-revs` above `stable`'s churn count (2) but at or below
    /// `hot`'s (6) filters `stable` out entirely via the `HAVING` floor,
    /// mirroring `hotspots`' `--min-revs` semantics.
    #[test]
    fn min_revs_filters_the_stable_sibling() {
        let rows = build_db_and_rows(5);

        assert!(
            rows.iter().any(|r| r.function.starts_with("hot@")),
            "hot (6 revs) must survive --min-revs 5"
        );
        assert!(
            !rows.iter().any(|r| r.function.starts_with("stable@")),
            "stable (2 revs) must be filtered out by --min-revs 5"
        );

        let hot = rows
            .iter()
            .find(|r| r.function.starts_with("hot@"))
            .expect("hot row");
        assert_eq!(hot.revs, 6);
    }

    /// A `--min-revs` above every function's churn count returns empty, not
    /// an error.
    #[test]
    fn min_revs_above_every_churn_count_returns_empty() {
        let rows = build_db_and_rows(7);
        assert!(
            rows.is_empty(),
            "expected empty rows at --min-revs 7 (max churn is 6); got {rows:?}"
        );
    }
}
