use std::str::FromStr;

use codelore_lib::Options;
use codelore_lib::analyses::ownership::run_ownership;
use codelore_lib::analysis::AnalysisName;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;

#[test]
fn ownership_single_author_has_zero_fragmentation() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_ownership(&db, &opts).expect("run");
    assert!(!rows.is_empty(), "ownership should produce ≥1 row");
    for row in &rows {
        // single author → HHI = 1, FV = 0
        assert!(
            row.fractal_value < 1e-9,
            "single-author file should have FV ≈ 0, got {} for {}",
            row.fractal_value,
            row.path
        );
        assert_eq!(row.main_author, "tiny@example.com", "tiny_repo author");
    }
}

/// `fragmentation` is code-maat's name for an analysis that emits
/// `entity, fractal-value, total-revs`. `CodeLore`'s `ownership` already
/// computes the same Herfindahl-Hirschman fractal value alongside a
/// `main-author` column, so the alias resolves to the same enum variant.
/// `code-ownership` is the name `CodeLore`'s own user-facing docs use to
/// disambiguate from `entity-ownership`; same target.
#[test]
fn ownership_accepts_fragmentation_and_code_ownership_aliases() {
    let canonical = AnalysisName::from_str("ownership").expect("canonical");
    for alias_name in ["fragmentation", "code-ownership"] {
        let resolved = AnalysisName::from_str(alias_name)
            .unwrap_or_else(|e| panic!("alias {alias_name:?} should resolve: {e}"));
        assert_eq!(canonical, resolved, "{alias_name} → ownership");
        assert!(matches!(resolved, AnalysisName::Ownership));
    }
}
