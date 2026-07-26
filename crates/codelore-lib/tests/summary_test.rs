use codelore_lib::Options;
use codelore_lib::analyses::summary::run_summary;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;
use std::path::Path;
use std::process::Command;

/// Commit one change to `file` in `dir` authored by `name <email>` with a
/// fixed date, so a fixture can mix human and bot authors deterministically.
fn commit_as(dir: &Path, name: &str, email: &str, file: &str, body: &str) {
    std::fs::write(dir.join(file), body).expect("write fixture file");
    let add = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["add", "."])
        .status()
        .expect("spawn git add");
    assert!(add.success(), "git add failed");
    let ok = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["commit", "--quiet", "-m", "change"])
        .env("GIT_AUTHOR_NAME", name)
        .env("GIT_AUTHOR_EMAIL", email)
        .env("GIT_COMMITTER_NAME", name)
        .env("GIT_COMMITTER_EMAIL", email)
        .env("GIT_AUTHOR_DATE", "2026-01-01T00:00:00Z")
        .env("GIT_COMMITTER_DATE", "2026-01-01T00:00:00Z")
        .status()
        .expect("spawn git commit")
        .success();
    assert!(ok, "git commit failed");
}

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

/// The modern `authors` count excludes bot identities, matching every other
/// social analysis — a CI bot is not a human contributor. Under
/// `--code-maat-compat` the count stays bot-inclusive, byte-faithful to
/// upstream code-maat (which has no bot concept), so downstream scripts
/// parsing the legacy CSV see the same value they always did.
#[test]
fn authors_count_excludes_bots_in_modern_but_not_compat() {
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    let init = Command::new("git")
        .arg("-C")
        .arg(p)
        .args(["init", "-b", "main", "--quiet"])
        .status()
        .expect("spawn git init");
    assert!(init.success(), "git init failed");

    // Two humans and one CI bot; the bot's `[bot]` identity is flagged at
    // ingest by the built-in bot heuristic.
    commit_as(p, "Alice", "alice@example.com", "a.rs", "fn a() {}\n");
    commit_as(p, "Bob", "bob@example.com", "b.rs", "fn b() {}\n");
    commit_as(
        p,
        "dependabot[bot]",
        "dependabot[bot]@users.noreply.github.com",
        "deps.rs",
        "// bump\n",
    );

    let repo = GixRepo::open(p).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: p.to_path_buf(),
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    // Precondition: the bot identity really was flagged, so the assertions
    // below exercise the filter rather than an empty bot set.
    let is_bot: bool = db
        .query_row(
            "SELECT COALESCE(BOOL_OR(is_bot), FALSE) FROM author_aliases \
             WHERE canonical LIKE '%dependabot%'",
            [],
            |r| r.get(0),
        )
        .expect("query is_bot");
    assert!(is_bot, "dependabot must be flagged is_bot at ingest");

    let modern = run_summary(&db, &opts).expect("run modern");
    let modern_authors = modern.iter().find(|r| r.metric == "authors").unwrap().value;
    assert_eq!(
        modern_authors, 2,
        "modern `authors` counts the two humans, not the bot: {modern:?}"
    );

    let compat_opts = Options {
        code_maat_compat: true,
        ..opts
    };
    let compat = run_summary(&db, &compat_opts).expect("run compat");
    let compat_authors = compat
        .iter()
        .find(|r| r.metric == "number-of-authors")
        .unwrap()
        .value;
    assert_eq!(
        compat_authors, 3,
        "--code-maat-compat stays bot-inclusive (byte-faithful to code-maat): {compat:?}"
    );
}
