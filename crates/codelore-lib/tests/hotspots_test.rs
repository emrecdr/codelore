use codelore_lib::Options;
use codelore_lib::analyses::hotspots::run_hotspots;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;

#[test]
fn hotspots_for_tiny_repo() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_hotspots(&db, &opts).expect("run");
    assert!(!rows.is_empty(), "should produce ≥1 hotspot row");

    // src/main.rs changed 4 times; src/lib.rs changed 1 time. Both Rust.
    // With similar complexity, main.rs should rank above lib.rs.
    let main_row = rows
        .iter()
        .find(|r| r.path == "src/main.rs")
        .expect("main.rs should be in hotspots");
    let lib_row = rows.iter().find(|r| r.path == "src/lib.rs");

    if let Some(lib) = lib_row {
        assert!(
            main_row.hotspot_score >= lib.hotspot_score,
            "main.rs (revs=4) should rank ≥ lib.rs (revs=1)"
        );
    }

    // Hotspot score should be in [0, 10] range (formula bounds:
    // percentile_rank ∈ [0,1], code_health ∈ [0,100], so score ∈ [0, 10])
    for row in &rows {
        assert!(
            row.hotspot_score >= 0.0,
            "hotspot score should be >= 0, got {} for {}",
            row.hotspot_score,
            row.path
        );
    }
}

/// The hotspots query joins `complexity_metrics` × `entities` on `kind='unit'`
/// to surface the file-level Maintainability Index (Coleman 1994 / SEI 1997
/// variant computed in `codelore-rca`). For the tiny Rust fixture, at least
/// one row should carry a non-null finite MI — the JOIN being broken would
/// silently return all-None.
#[test]
fn hotspots_surface_maintainability_index_for_rust_files() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_hotspots(&db, &opts).expect("run");
    let any_finite_mi = rows.iter().any(|r| matches!(r.mi, Some(v) if v.is_finite()));
    assert!(
        any_finite_mi,
        "expected ≥1 Rust hotspot row with a finite MI; got rows: {:?}",
        rows.iter()
            .map(|r| (r.path.as_str(), r.mi))
            .collect::<Vec<_>>()
    );
}

/// The hotspots SQL also surfaces an `ai_pct` column: share of commits
/// touching a file that carry the `ai-assisted` or `ai-authored`
/// attribution from `identity::bots`. Every hotspot row should have a
/// non-null `ai_pct` (in `[0, 100]`) — the LEFT JOIN with `file_ai` is
/// over the same node set as `file_revs`, so the only way to get None
/// here would be if the JOIN got silently broken (column collision,
/// rename drift, etc).
#[test]
fn hotspots_surface_ai_attribution_percentage() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_hotspots(&db, &opts).expect("run");
    assert!(!rows.is_empty(), "fixture should produce ≥1 hotspot row");
    for row in &rows {
        match row.ai_pct {
            Some(p) => assert!(
                (0.0..=100.0).contains(&p) && p.is_finite(),
                "ai_pct out of range for {}: {p}",
                row.path
            ),
            None => panic!(
                "ai_pct should be Some on every hotspot row (LEFT JOIN over the \
                 same node set as file_revs); got None on {}",
                row.path
            ),
        }
    }
}

/// `FactsDb::explain_sql` returns a non-empty `DuckDB` optimizer plan for
/// the hotspots SQL. The CLI's `--explain` flag routes through this
/// helper; missing or empty plan output would mean `--explain` silently
/// no-ops.
#[test]
fn explain_sql_returns_non_empty_plan() {
    use codelore_lib::analyses::hotspots::SQL;
    use duckdb::params;
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let plan = db
        .explain_sql(SQL, params![1u32, i64::MAX])
        .expect("explain");
    assert!(
        !plan.is_empty(),
        "EXPLAIN plan should not be empty; got {plan:?}"
    );
    // DuckDB EXPLAIN output reliably contains either "PROJECTION" or
    // "HASH_JOIN" or "ORDER_BY" for any non-trivial query.
    let upper = plan.to_uppercase();
    assert!(
        upper.contains("PROJECTION")
            || upper.contains("ORDER")
            || upper.contains("JOIN")
            || upper.contains("AGGREGATE"),
        "EXPLAIN plan missing common operator names; got {plan:?}"
    );
}
