use codelore_lib::Options;
use codelore_lib::facts::FactsDb;
use codelore_lib::output::sqlite;
use codelore_lib::repo::GixRepo;
use duckdb::Connection;

/// Base tables the `SQLite` export must dump, derived from the schema itself so
/// a new `CREATE TABLE` added to `schema_v1.sql` that the export forgets to
/// dump fails this test instead of silently dropping data downstream.
fn schema_base_tables() -> Vec<String> {
    const SCHEMA: &str = include_str!("../src/facts/schema_v1.sql");
    SCHEMA
        .lines()
        .filter_map(|line| {
            let rest = line.trim().strip_prefix("CREATE TABLE IF NOT EXISTS ")?;
            let name = rest.split([' ', '(']).next()?;
            (!name.is_empty()).then(|| name.to_string())
        })
        .collect()
}

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

    // Every base table in schema_v1.sql must round-trip. The clones,
    // imports, and commit_parents tables were each silently omitted at some
    // point; deriving the expected set from the schema (rather than a
    // hand-maintained list that drifts the same way the export does) makes
    // the next dropped table fail here.
    let expected = schema_base_tables();
    assert!(
        expected.len() >= 10,
        "schema parse found too few base tables ({}): {expected:?}",
        expected.len()
    );
    for table in &expected {
        let sql = format!("SELECT COUNT(*) FROM db.{table}");
        reader
            .query_row::<i64, _, _>(&sql, [], |r| r.get(0))
            .unwrap_or_else(|e| panic!("table {table} missing from sqlite dump: {e}"));
    }
}

/// A failure to load the sqlite extension must name its two prerequisites.
///
/// `INSTALL sqlite` is the one step of the export that reaches outside the
/// process: it fetches the extension and caches it under `DuckDB`'s home
/// directory. Both an air-gapped host and a locked-down home land here, and the
/// bare `DuckDB` error names neither cause — it reads as a bug in the export
/// rather than a missing prerequisite. That is why `INSTALL`/`LOAD` is issued
/// as its own statement: the hint attaches to the step it explains, with no
/// pattern matching against `DuckDB`'s error text.
///
/// The failure is induced by pointing `DuckDB`'s own `home_directory` at an
/// unwritable path, so the test needs neither network isolation nor a mutation
/// of the process environment. Unix-only because it relies on `chmod` bits,
/// which do not express "unwritable" the same way on Windows.
#[cfg(unix)]
#[test]
fn sqlite_extension_failure_explains_its_prerequisites() {
    use std::os::unix::fs::PermissionsExt as _;

    let repo = codelore_lib::test_support::tiny_repo::build();
    let opts = Options {
        repo_path: repo.dir.path().to_path_buf(),
        ..Options::default()
    };
    let gix = GixRepo::open(repo.dir.path()).expect("open fixture repo");
    let db = FactsDb::open_or_ingest(&opts, &gix).expect("ingest fixture");

    let home = repo.dir.path().join("locked-home");
    std::fs::create_dir(&home).expect("create locked home");
    std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o500))
        .expect("make home unwritable");
    let home_sql = home.display().to_string().replace('\'', "''");
    db.execute_batch(&format!("SET home_directory='{home_sql}';"))
        .expect("point DuckDB at the locked home");

    let out = repo.dir.path().join("dump.sqlite");
    let err = sqlite::write_full_fact_store_sqlite(&db, &opts, &out)
        .expect_err("an unwritable extension home must fail the export");
    let _ = std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o700));

    let msg = err.to_string();
    assert!(
        msg.contains("hint:"),
        "an extension-load failure must carry a hint, got: {msg}"
    );
    for expected in ["network access", "writable cache", ".duckdb/extensions"] {
        assert!(
            msg.contains(expected),
            "the hint must name {expected:?} — the two prerequisites and where the \
             cache lives are what make it actionable. Got: {msg}"
        );
    }
}
