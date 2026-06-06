use codelore_lib::Options;
use codelore_lib::facts::FactsDb;
use codelore_lib::output::parquet;
use codelore_lib::repo::GixRepo;
use duckdb::Connection;

#[test]
fn parquet_hotspots_roundtrip() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("hotspots.parquet");
    parquet::write_hotspots_parquet(&db, &opts, &path).expect("write");

    // Read back via DuckDB and assert ≥1 row
    let reader = Connection::open_in_memory().expect("open");
    let path_str = path.display().to_string();
    let count: i64 = reader
        .query_row(&format!("SELECT COUNT(*) FROM '{path_str}'"), [], |r| {
            r.get(0)
        })
        .expect("count");
    assert!(
        count >= 1,
        "expected at least 1 hotspot row in parquet, got {count}"
    );
}
