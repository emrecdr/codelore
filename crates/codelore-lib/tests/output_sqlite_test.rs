use codelore_lib::Options;
use codelore_lib::facts::FactsDb;
use codelore_lib::output::sqlite;
use codelore_lib::repo::GixRepo;
use duckdb::Connection;

#[test]
fn sqlite_full_dump_roundtrip() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("dump.db");
    sqlite::write_full_fact_store_sqlite(&db, &opts, &path).expect("write");

    assert!(path.exists(), "sqlite file should be created");

    let reader = Connection::open_in_memory().expect("open");
    let path_str = path.display().to_string();
    reader
        .execute_batch(&format!(
            "INSTALL sqlite; LOAD sqlite; ATTACH '{path_str}' AS db (TYPE SQLITE);"
        ))
        .expect("attach");
    let commit_count: i64 = reader
        .query_row("SELECT COUNT(*) FROM db.commits", [], |r| r.get(0))
        .expect("count");
    assert_eq!(commit_count, 5, "tiny_repo has 5 commits");

    // Every base table in schema_v1.sql must round-trip. The clones table
    // was silently omitted before this regression test was added.
    for table in [
        "commits",
        "changes",
        "hunks",
        "entities",
        "complexity_metrics",
        "author_aliases",
        "provenance",
        "clones",
    ] {
        let sql = format!("SELECT COUNT(*) FROM db.{table}");
        reader
            .query_row::<i64, _, _>(&sql, [], |r| r.get(0))
            .unwrap_or_else(|e| panic!("table {table} missing from sqlite dump: {e}"));
    }
}
