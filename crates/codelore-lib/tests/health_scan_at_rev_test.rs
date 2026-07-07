//! Integration tests for the rev-parameterized ingest helpers.
//!
//! Verifies that `ingest_complexity_at_rev` produces the same
//! `(path, name, cyclomatic, loc)` rows as the HEAD complexity scan for
//! the same tree, and that `materialize_imports_at_rev` writes the expected
//! edge rows for a known import graph.

use codelore_lib::Options;
use codelore_lib::analyses::import_graph::build_import_graph_from_edges;
use codelore_lib::facts::FactsDb;
use codelore_lib::facts::ingest::{ingest_complexity_at_rev, materialize_imports_at_rev};
use codelore_lib::repo::GixRepo;

/// Returns all `(path, name, cyclomatic, loc)` rows from `table_name`,
/// ordered by `(path, name)`.
fn collect_metric_rows(
    db: &FactsDb,
    table_name: &str,
) -> Vec<(String, String, Option<i32>, Option<i32>)> {
    let sql = format!("SELECT path, name, cyclomatic, loc FROM {table_name} ORDER BY path, name");
    let mut stmt = db.prepare(&sql).expect("prepare metric query");
    stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, Option<i32>>(2)?,
            r.get::<_, Option<i32>>(3)?,
        ))
    })
    .expect("query metric rows")
    .collect::<Result<Vec<_>, _>>()
    .expect("collect metric rows")
}

/// Returns the live-at-HEAD path set from the fact store using the same
/// `arg_max` CTE as the internal `query_live_paths`.
fn live_paths_from_db(db: &FactsDb) -> Vec<String> {
    let sql = "
        WITH latest_per_path AS (
            SELECT
                c.path,
                arg_max(
                    c.change_type,
                    ROW(commits.date, -commits.rowid)
                ) AS change_type
            FROM changes c
            INNER JOIN commits ON commits.rev = c.rev
            GROUP BY c.path
        )
        SELECT path
        FROM latest_per_path
        WHERE change_type != 'deleted'
        ORDER BY path
    ";
    let mut stmt = db.prepare(sql).expect("prepare paths query");
    stmt.query_map([], |r| r.get::<_, String>(0))
        .expect("query paths")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect paths")
}

#[test]
fn ingest_complexity_at_rev_matches_head() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open repo");
    let db = FactsDb::new_in_memory().expect("new db");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let head_rows = collect_metric_rows(&db, "complexity_metrics");
    assert!(
        !head_rows.is_empty(),
        "HEAD complexity scan must produce at least one row"
    );

    let live_paths = live_paths_from_db(&db);
    let head_rev = &tiny.head_sha;

    ingest_complexity_at_rev(&db, &repo, head_rev, &live_paths, "cm_at_rev")
        .expect("ingest_complexity_at_rev");

    let at_rev_rows = collect_metric_rows(&db, "cm_at_rev");

    assert_eq!(
        head_rows, at_rev_rows,
        "ingest_complexity_at_rev at HEAD should produce the same rows as the HEAD complexity scan"
    );
}

#[test]
fn materialize_imports_at_rev_writes_edge_rows() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open repo");
    let db = FactsDb::new_in_memory().expect("new db");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    // Build a known import graph: main.rs → lib.rs.
    let edges = vec![("src/main.rs".to_string(), "src/lib.rs".to_string())];
    let graph = build_import_graph_from_edges(&edges);

    materialize_imports_at_rev(&db, &graph, "im_at_rev").expect("materialize_imports_at_rev");

    let mut stmt = db
        .prepare("SELECT src_path, target_path FROM im_at_rev ORDER BY src_path")
        .expect("prepare im query");
    let rows: Vec<(String, Option<String>)> = stmt
        .query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
        })
        .expect("query im rows")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect im rows");

    assert_eq!(rows.len(), 1, "expected exactly 1 import edge");
    assert_eq!(rows[0].0, "src/main.rs", "src_path mismatch");
    assert_eq!(
        rows[0].1.as_deref(),
        Some("src/lib.rs"),
        "target_path mismatch"
    );
}

#[test]
fn materialize_imports_at_rev_empty_graph_produces_no_rows() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open repo");
    let db = FactsDb::new_in_memory().expect("new db");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let graph = build_import_graph_from_edges(&[]);
    materialize_imports_at_rev(&db, &graph, "im_empty").expect("materialize empty graph");

    let count: i64 = db
        .query_row("SELECT COUNT(*) FROM im_empty", [], |r| r.get(0))
        .expect("count im_empty");
    assert_eq!(count, 0, "empty graph should produce zero rows");
}
