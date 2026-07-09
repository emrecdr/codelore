//! Integration tests for the `code-familiarity` analysis.
//!
//! Uses `delivery_repo` (3 authors, known commit timeline) and `tiny_repo`
//! (single author) to verify row shape, score bounds, and the hand-computed
//! expected familiarity value.
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
/// Expected: 1 row, `active_authors` = 3, `total_authors` = 3.
///
/// Hand-computed familiarity:
///   The `k_norm` share per (file, author) normalises to sum = 1.0 per file
///   (enforced by the SQL in `materialize_knowledge_shares`). Because all
///   3 authors are in the active window, `active_k_sum` per file = sum of
///   all `k_norm` values for that file = 1.0. The SLOC-weighted formula is:
///
///     familiarity = Σ_f (sloc_f × active_k_sum_f) / Σ_f sloc_f
///                 = Σ_f (sloc_f × 1.0) / Σ_f sloc_f
///                 = 100%
///
///   This holds for any SLOC distribution, so the expected value is exactly
///   100.0 whenever the entire author set is active. We assert within ±0.5
///   to tolerate any floating-point rounding in the accumulation.
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

/// All 3 authors active within the 90-day window →  familiarity = 100%.
///
/// Hand-computed: since `active_k_sum` = 1.0 for every file (all authors
/// active, `k_norm` sums to 1.0 per file), the SLOC-weighted score is
/// 100.0 regardless of per-file SLOC. Expected within ±0.5 tolerance.
#[test]
fn delivery_repo_familiarity_is_100_pct_all_authors_active() {
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
    assert_eq!(rows.len(), 1, "should return one row");
    let row = &rows[0];
    assert_eq!(
        row.active_authors, 3,
        "all 3 authors must be active within 90 days"
    );
    assert_eq!(row.total_authors, 3, "delivery_repo has exactly 3 authors");
    // When all authors are active, active_k_sum per file = 1.0, so
    // familiarity = 100.0 exactly (modulo floating-point rounding).
    assert!(
        (row.familiarity_pct - 100.0).abs() < 0.5,
        "expected familiarity ≈ 100.0 (all authors active), got {:.4}",
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
    assert_eq!(rows.len(), 1, "should return one row");
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
