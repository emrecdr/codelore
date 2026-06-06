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
    assert_eq!(v, "1");
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
        assert_eq!(schema_version, "1");
    }
}
