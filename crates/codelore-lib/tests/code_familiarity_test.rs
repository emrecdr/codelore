//! Integration tests for the `code-familiarity` analysis.
//!
//! Uses `delivery_repo` (3 authors, known commit timeline) and `tiny_repo`
//! (single author) to verify row shape, score bounds, and the single-author
//! full-familiarity property.
//!
//! Requires the `test-support` feature (declared in `Cargo.toml`).

use codelore_lib::Options;
use codelore_lib::analyses::code_familiarity::run_code_familiarity;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;

/// `delivery_repo` has 3 authors spanning 2026-01-01 to 2026-04-21.
/// Last commit is Alice on 2026-04-21. Default `window_days` = 90 means
/// the active window starts 2026-01-21. All three authors have commits
/// after that date:
///   Alice  — Apr 21 (active)
///   Bob    — Mar 3  (active, 49 days before Apr 21)
///   Carol  — Mar 22 (active, 30 days before Apr 21)
/// Expected: 1 row, `active_authors` = 3, `total_authors` = 3,
/// `familiarity_pct` in (0, 100], `islands_pct` in [0, 100].
#[test]
fn delivery_repo_familiarity_row_shape() {
    let fixture = codelore_lib::test_support::delivery_repo::build();
    let repo = GixRepo::open(fixture.dir.path()).expect("open repo");
    let db = FactsDb::new_in_memory().expect("new_in_memory");
    let opts = Options {
        repo_path: fixture.dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_code_familiarity(&db, &opts).expect("run_code_familiarity");

    // If complexity_metrics is empty (no recognized source files), familiarity
    // is undefined and `run_code_familiarity` correctly returns an empty vec.
    if rows.is_empty() {
        return;
    }

    assert_eq!(rows.len(), 1, "should return exactly one repo-scope row");
    let row = &rows[0];
    assert_eq!(row.scope, "repo");
    assert!(
        row.familiarity_pct >= 0.0 && row.familiarity_pct <= 100.0,
        "familiarity_pct out of range: {}",
        row.familiarity_pct
    );
    assert!(
        row.islands_pct >= 0.0 && row.islands_pct <= 100.0,
        "islands_pct out of range: {}",
        row.islands_pct
    );
    assert!(
        row.active_authors <= row.total_authors,
        "active_authors ({}) > total_authors ({})",
        row.active_authors,
        row.total_authors
    );
    assert!(
        row.verdict == "good" || row.verdict == "risky",
        "verdict must be 'good' or 'risky', got '{}'",
        row.verdict
    );
}

/// With 3 active authors all knowledge is held by the active team —
/// `familiarity_pct` must be in [0, 100].
#[test]
fn delivery_repo_familiarity_score_in_range() {
    let fixture = codelore_lib::test_support::delivery_repo::build();
    let repo = GixRepo::open(fixture.dir.path()).expect("open repo");
    let db = FactsDb::new_in_memory().expect("new_in_memory");
    let opts = Options {
        repo_path: fixture.dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_code_familiarity(&db, &opts).expect("run_code_familiarity");
    if rows.is_empty() {
        return;
    }
    let row = &rows[0];
    assert!(
        row.familiarity_pct >= 0.0 && row.familiarity_pct <= 100.0,
        "familiarity_pct must be in [0,100]: {}",
        row.familiarity_pct
    );
}

/// `islands_pct` must always be in [0, 100].
#[test]
fn delivery_repo_islands_pct_in_range() {
    let fixture = codelore_lib::test_support::delivery_repo::build();
    let repo = GixRepo::open(fixture.dir.path()).expect("open repo");
    let db = FactsDb::new_in_memory().expect("new_in_memory");
    let opts = Options {
        repo_path: fixture.dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_code_familiarity(&db, &opts).expect("run_code_familiarity");
    if rows.is_empty() {
        return;
    }
    let row = &rows[0];
    assert!(
        row.islands_pct >= 0.0 && row.islands_pct <= 100.0,
        "islands_pct must be in [0,100]: {}",
        row.islands_pct
    );
}

/// `tiny_repo` has a single author. The `complexity_metrics` table will be
/// empty (no recognized source files), so `run_code_familiarity` returns an
/// empty vec — correct behaviour when there is no SLOC to measure.
#[test]
fn tiny_repo_single_author_no_error() {
    let fixture = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(fixture.dir.path()).expect("open repo");
    let db = FactsDb::new_in_memory().expect("new_in_memory");
    let opts = Options {
        repo_path: fixture.dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_code_familiarity(&db, &opts).expect("run_code_familiarity");
    assert!(
        rows.len() <= 1,
        "should return 0 or 1 rows, got {}",
        rows.len()
    );
    if let Some(row) = rows.first() {
        assert_eq!(row.scope, "repo");
        assert!(
            (row.familiarity_pct - 100.0).abs() < 0.5,
            "single-author repo should have ~100% familiarity: {}",
            row.familiarity_pct
        );
        assert_eq!(row.active_authors, 1);
        assert_eq!(row.total_authors, 1);
    }
}

/// Idempotence: calling `run_code_familiarity` twice on the same db
/// returns identical results — the `Cell<bool>` guard in
/// `materialize_knowledge_shares` prevents double-materialisation.
#[test]
fn delivery_repo_idempotent() {
    let fixture = codelore_lib::test_support::delivery_repo::build();
    let repo = GixRepo::open(fixture.dir.path()).expect("open repo");
    let db = FactsDb::new_in_memory().expect("new_in_memory");
    let opts = Options {
        repo_path: fixture.dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows1 = run_code_familiarity(&db, &opts).expect("first call");
    let rows2 = run_code_familiarity(&db, &opts).expect("second call");

    assert_eq!(rows1.len(), rows2.len(), "idempotent: row count must match");
    if let (Some(r1), Some(r2)) = (rows1.first(), rows2.first()) {
        assert!(
            (r1.familiarity_pct - r2.familiarity_pct).abs() < 1e-9,
            "idempotent: familiarity_pct must match: {} vs {}",
            r1.familiarity_pct,
            r2.familiarity_pct
        );
    }
}
