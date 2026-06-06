use bca_lib::facts::FactsDb;

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
