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
    // Assert against the constant, not a literal: schema bumps must not
    // require hand-editing this test (the literal drifted twice before).
    assert_eq!(v, codelore_lib::facts::schema::CURRENT_SCHEMA_VERSION);
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
        db.execute_batch("UPDATE provenance SET value = '999' WHERE key = 'schema_version'")
            .expect("bump schema_version");
    }
    let msg = match FactsDb::open_read_only(&path) {
        Ok(_) => panic!("stale schema should fail open_read_only"),
        Err(e) => format!("{e}"),
    };
    let expects = format!(
        "expects {}",
        codelore_lib::facts::schema::CURRENT_SCHEMA_VERSION
    );
    assert!(
        msg.contains("schema_version=999") && msg.contains(&expects),
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
fn low_memory_limit_spills_to_temp_dir_instead_of_erroring() {
    // Mirror the shape of a real ingest: a bulk `CREATE TABLE AS SELECT`
    // that just accumulates rows (no ORDER BY / sort operator involved) —
    // the same pattern the `Appender`-driven fact-table population uses.
    // 5M rows of 4 `DOUBLE` columns is ~160 MB of raw column data, which
    // doesn't fit under a 128 MB ceiling, so `DuckDB` must spill the
    // growing table's blocks to `temp_directory` rather than erroring.
    let dir = tempfile::tempdir().expect("tempdir");
    let spill_dir = dir.path().join("spill");

    let db = FactsDb::new_in_memory_with_memory_limit("128MB", &spill_dir)
        .expect("in-memory db with a low memory_limit must still open + create schema");

    db.execute_batch(
        "CREATE TABLE spill_probe AS \
         SELECT i, random() AS a, random() AS b, random() AS c, random() AS d \
         FROM range(5000000) AS t(i);",
    )
    .expect("bulk table load must succeed by spilling, not by erroring/OOM-ing");

    let count: i64 = db
        .query_row("SELECT count(*) FROM spill_probe", [], |r| r.get(0))
        .expect("count spilled rows");
    assert_eq!(
        count, 5_000_000,
        "spilled table load must still see every row"
    );

    assert!(
        spill_dir.is_dir(),
        "temp_directory PRAGMA target must exist: {}",
        spill_dir.display()
    );
    let spilled_files = std::fs::read_dir(&spill_dir)
        .expect("read spill dir")
        .count();
    assert!(
        spilled_files > 0,
        "expected DuckDB to write spill file(s) under temp_dir at {}; found none — \
         the memory_limit may not have been low enough to force an external sort",
        spill_dir.display()
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
        assert_eq!(
            schema_version,
            codelore_lib::facts::schema::CURRENT_SCHEMA_VERSION
        );
    }
}
