use std::path::Path;

use codelore_lib::Options;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;

fn run_git(path: &Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(path)
        .args(args)
        .status()
        .expect("git");
    assert!(status.success(), "git {args:?} failed");
}

fn write_file(root: &Path, rel: &str, content: &str) {
    let p = root.join(rel);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(p, content).unwrap();
}

#[test]
fn ingest_classifies_bot_commits_as_ai_authored() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path();

    run_git(path, &["init", "-b", "main", "--quiet"]);
    run_git(
        path,
        &["config", "user.email", "dependabot[bot]@noreply.github.com"],
    );
    run_git(path, &["config", "user.name", "dependabot[bot]"]);

    write_file(
        path,
        "Cargo.toml",
        "[package]\nname = \"x\"\nversion = \"0.1.0\"\n",
    );
    run_git(path, &["add", "."]);
    run_git(path, &["commit", "-m", "bump deps", "--quiet"]);

    let repo = GixRepo::open(path).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options::default();
    db.ingest(&repo, &opts).expect("ingest");

    let ai_attr: String = db
        .query_one_value("SELECT ai_attribution FROM commits LIMIT 1")
        .expect("ai_attribution query");
    assert_eq!(
        ai_attr, "ai-authored",
        "dependabot commits should be ai-authored"
    );

    let is_bot: String = db
        .query_one_value(
            "SELECT CAST(is_bot AS TEXT) FROM author_aliases WHERE raw_email = 'dependabot[bot]@noreply.github.com'",
        )
        .expect("is_bot query");
    assert_eq!(
        is_bot, "true",
        "dependabot should be flagged as bot in author_aliases"
    );
}

#[test]
fn ingest_classifies_human_commits_correctly() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options::default();
    db.ingest(&repo, &opts).expect("ingest");

    // All tiny_repo commits are by a human author
    let ai_attr_values: String = db
        .query_one_value(
            "SELECT CAST(COUNT(*) AS TEXT) FROM commits WHERE ai_attribution = 'human'",
        )
        .expect("human count query");
    let count: u32 = ai_attr_values.parse().unwrap();
    assert_eq!(
        count, 5,
        "all 5 tiny repo commits should be classified as human"
    );
}

#[test]
fn ingest_populates_author_aliases() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options::default();
    db.ingest(&repo, &opts).expect("ingest");

    // tiny_repo uses "tiny@example.com" as the author
    let alias_count: String = db
        .query_one_value(
            "SELECT CAST(COUNT(*) AS TEXT) FROM author_aliases WHERE raw_email = 'tiny@example.com'",
        )
        .expect("alias count query");
    let n: u32 = alias_count.parse().unwrap();
    assert_eq!(n, 1, "expected one alias row for tiny@example.com");
}

#[test]
fn ingest_tiny_repo_writes_5_commits() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");

    let opts = Options::default();
    let n = db.ingest(&repo, &opts).expect("ingest");
    assert_eq!(n.commits_ingested, 5);

    let count: String = db
        .query_one_value("SELECT CAST(COUNT(*) AS TEXT) FROM commits")
        .expect("count");
    assert_eq!(count, "5");
}

#[test]
fn ingest_populates_complexity_for_tier1_files() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");

    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let entity_count: String = db
        .query_one_value("SELECT CAST(COUNT(*) AS TEXT) FROM entities WHERE path = 'src/main.rs'")
        .expect("entity count query");
    let n: u32 = entity_count.parse().unwrap();
    assert!(n >= 1, "expected ≥1 entity for src/main.rs, got {n}");

    let metric_count: String = db
        .query_one_value(
            "SELECT CAST(COUNT(*) AS TEXT) FROM complexity_metrics WHERE path = 'src/main.rs'",
        )
        .expect("metric count query");
    let m: u32 = metric_count.parse().unwrap();
    assert!(
        m >= 1,
        "expected ≥1 complexity row for src/main.rs, got {m}"
    );
}

/// Regression test for F149. Pre-fix the `hunks` table existed in
/// schema but `append_change` never wrote to it — `Repo::diff_hunks`
/// parsed the headers, attached them to `FileChange.hunks`, and the
/// payload was dropped on the floor. The new ingest path writes one
/// row per hunk; this test makes two edits to the same file in
/// separate, non-adjacent regions (the gix diff splits them into two
/// hunks) and asserts both land in the table with the schema-required
/// NOT NULL fields and unique composite key.
#[test]
fn ingest_writes_hunk_rows_to_hunks_table() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path();
    run_git(path, &["init", "-b", "main", "--quiet"]);
    run_git(path, &["config", "user.email", "alice@example.com"]);
    run_git(path, &["config", "user.name", "Alice"]);

    // Initial file with a 30-line body — two later edits will land in
    // separate hunks (gix's diff coalesces only within ~3-line
    // proximity, so an edit at line 2 and an edit at line 25 are
    // guaranteed to surface as two distinct `@@ … @@` headers).
    let initial: String = (1..=30)
        .map(|n| format!("line {n}"))
        .collect::<Vec<_>>()
        .join("\n");
    write_file(path, "src/code.rs", &format!("{initial}\n"));
    run_git(path, &["add", "."]);
    run_git(path, &["commit", "-m", "initial", "--quiet"]);

    // Edit lines 2 and 25 — far enough apart that gix emits two hunks.
    let edited: String = (1..=30)
        .map(|n| match n {
            2 => "line 2 EDITED".to_string(),
            25 => "line 25 EDITED".to_string(),
            other => format!("line {other}"),
        })
        .collect::<Vec<_>>()
        .join("\n");
    write_file(path, "src/code.rs", &format!("{edited}\n"));
    run_git(path, &["commit", "-am", "two non-adjacent edits", "--quiet"]);

    let repo = GixRepo::open(path).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options::default();
    db.ingest(&repo, &opts).expect("ingest");

    // Pre-fix this returned 0 — `append_change` never wrote a hunks
    // row, even though `FileChange.hunks` carried the parsed payload.
    let hunk_count: String = db
        .query_one_value(
            "SELECT CAST(COUNT(*) AS TEXT) FROM hunks WHERE path = 'src/code.rs'",
        )
        .expect("hunk count query");
    let n: u32 = hunk_count.parse().unwrap();
    assert!(
        n >= 2,
        "expected ≥2 hunk rows for the two-edit commit on src/code.rs, got {n}"
    );

    // Verify schema v4 NOT NULL invariant: every persisted hunk row
    // has all four offsets populated (the parser drops malformed
    // hunks, and `Hunk` fields are u32 in Rust, so NULLs would only
    // happen if a future code path bypassed the type system).
    let nulls: String = db
        .query_one_value(
            "SELECT CAST(COUNT(*) AS TEXT) FROM hunks \
             WHERE old_start IS NULL OR old_lines IS NULL \
                OR new_start IS NULL OR new_lines IS NULL",
        )
        .expect("null offsets query");
    assert_eq!(nulls, "0", "no hunk row may have NULL offset columns");
}
