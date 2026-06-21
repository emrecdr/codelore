use codelore_lib::facts::FactsDb;

#[test]
fn creates_v1_schema() {
    let db = FactsDb::new_in_memory().expect("create");
    let tables = db.list_tables().expect("list");
    let expected = [
        "commits",
        "changes",
        "hunks",
        "entities",
        "complexity_metrics",
        "author_aliases",
        "provenance",
    ];
    for t in &expected {
        assert!(tables.iter().any(|n| n == t), "table {t} missing");
    }
}

#[test]
fn provenance_records_schema_version() {
    let db = FactsDb::new_in_memory().expect("create");
    let v: String = db
        .query_one_value("SELECT value FROM provenance WHERE key = 'schema_version'")
        .expect("query");
    assert_eq!(v, "3");
}

#[test]
fn open_read_only_validates_schema_version_match() {
    // Happy path: a fact store ingested by the current binary should
    // re-open in read-only mode without error.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("ok.duckdb");
    {
        let _ = FactsDb::open(&path).expect("create");
    }
    let reopen = FactsDb::open_read_only(&path);
    assert!(
        reopen.is_ok(),
        "open_read_only on current schema should succeed"
    );
}

#[test]
fn open_read_only_rejects_mismatched_schema_version() {
    // Simulate an operator handing a stale .duckdb to `--cache-dir`:
    // create the fact store, then bump the stored schema_version to a
    // value the current binary doesn't expect. open_read_only must
    // surface a typed parse-time error instead of bubbling cryptic
    // Catalog Errors at analysis time.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("stale.duckdb");
    {
        let db = FactsDb::open(&path).expect("create");
        db.conn()
            .execute_batch("UPDATE provenance SET value = '999' WHERE key = 'schema_version'")
            .expect("bump schema_version");
    }
    let msg = match FactsDb::open_read_only(&path) {
        Ok(_) => panic!("stale schema should fail open_read_only"),
        Err(e) => format!("{e}"),
    };
    assert!(
        msg.contains("schema_version=999") && msg.contains("expects 3"),
        "expected mismatch error, got: {msg}"
    );
}

#[test]
fn open_read_only_rejects_non_factstore_file() {
    // A random DuckDB file with no provenance table should also fail.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("foreign.duckdb");
    {
        let conn = duckdb::Connection::open(&path).expect("open foreign");
        conn.execute_batch("CREATE TABLE foo(x INTEGER)")
            .expect("create foreign schema");
    }
    let msg = match FactsDb::open_read_only(&path) {
        Ok(_) => panic!("foreign DB should fail open_read_only"),
        Err(e) => format!("{e}"),
    };
    assert!(
        msg.contains("missing the provenance schema_version row"),
        "expected missing-row error, got: {msg}"
    );
}

#[test]
fn file_backed_db_persists_and_reopens() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("test.duckdb");

    // First open: create + write schema
    {
        let db = FactsDb::open(&path).expect("create file-backed");
        let tables = db.list_tables().expect("list");
        assert!(
            tables.iter().any(|n| n == "commits"),
            "commits table should exist"
        );
    }

    // Second open: same path should re-open without re-creating (CREATE IF NOT EXISTS)
    {
        let db = FactsDb::open(&path).expect("reopen file-backed");
        let schema_version: String = db
            .query_one_value("SELECT value FROM provenance WHERE key = 'schema_version'")
            .expect("query");
        assert_eq!(schema_version, "3");
    }
}
