use codelore_lib::Options;
use codelore_lib::analyses::summary::run_summary;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;

#[test]
fn summary_for_tiny_repo() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_summary(&db, &opts).expect("run");
    assert_eq!(rows.len(), 4, "summary should produce exactly 4 rows");

    let commits = rows.iter().find(|r| r.metric == "commits").unwrap();
    assert_eq!(commits.value, 5, "tiny_repo has 5 commits");

    let authors = rows.iter().find(|r| r.metric == "authors").unwrap();
    assert_eq!(authors.value, 1, "tiny_repo has 1 author");
}

/// Under `--code-maat-compat`, `number-of-entities` counts distinct CHANGED
/// file paths (code-maat's semantic), not tree-sitter functions/classes. In
/// `tiny_repo`, `src/main.rs` (4 commits) + `src/lib.rs` (1 commit) means 2
/// distinct changed paths, 5 change records, 5 commits, 1 author.
#[test]
fn summary_number_of_entities_counts_changed_paths_under_compat() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        code_maat_compat: true,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");
    let rows = run_summary(&db, &opts).expect("run");
    let get = |m: &str| rows.iter().find(|r| r.metric == m).expect("metric").value;
    assert_eq!(
        get("number-of-entities"),
        2,
        "compat = distinct changed paths, not the 4-row entities table"
    );
    assert_eq!(get("number-of-entities-changed"), 5);
    assert_eq!(get("number-of-commits"), 5);
    assert_eq!(get("number-of-authors"), 1);
}
