//! Validation of the recursive rename-lineage CTE.
//!
//! The `differential_repo` fixture renames `src/old_name.rs` → `src/new_name.rs`
//! once (commit `546f33a`) and then `src/new_name.rs` gets one more
//! revision (`later change to src/new_name.rs`). The pre-rename file had
//! at least one prior commit too. Without canonical lineage, `revisions`
//! reports TWO entities (`old_name.rs` and `new_name.rs`) with split
//! counts. With lineage, they merge under `new_name.rs`.

use codelore_lib::Options;
use codelore_lib::analyses::revisions::run_revisions;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;

#[test]
fn rename_history_merges_under_canonical_path_when_lineage_on() {
    let diff = codelore_lib::test_support::differential_repo::build();
    let repo = GixRepo::open(diff.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");

    let opts_on = Options {
        repo_path: diff.dir.path().to_path_buf(),
        min_revs: 1,
        use_canonical_lineage: true,
        ..Options::default()
    };
    db.ingest(&repo, &opts_on).expect("ingest");
    let rows_on = run_revisions(&db, &opts_on).expect("revisions");

    // With lineage on: the OLD path must NOT appear in the output
    // because all its revisions have been folded into the new path.
    let old_appears = rows_on.iter().any(|(path, _)| path.contains("old_name"));
    assert!(
        !old_appears,
        "old_name.rs should NOT appear when canonical_lineage is on; got rows: {rows_on:?}"
    );
    // And the new path SHOULD appear with the merged history.
    let new_count = rows_on
        .iter()
        .find(|(p, _)| p.contains("new_name"))
        .map(|(_, n)| *n);
    assert!(
        new_count.is_some_and(|n| n >= 2),
        "new_name.rs should aggregate ≥2 revisions (pre-rename + post-rename); got: {rows_on:?}"
    );
}

#[test]
fn split_history_returns_when_lineage_off() {
    let diff = codelore_lib::test_support::differential_repo::build();
    let repo = GixRepo::open(diff.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");

    let opts_off = Options {
        repo_path: diff.dir.path().to_path_buf(),
        min_revs: 1,
        use_canonical_lineage: false,
        ..Options::default()
    };
    db.ingest(&repo, &opts_off).expect("ingest");
    let rows_off = run_revisions(&db, &opts_off).expect("revisions");

    // With lineage off: both paths appear (code-maat-parity behaviour).
    let old_appears = rows_off.iter().any(|(path, _)| path.contains("old_name"));
    let new_appears = rows_off.iter().any(|(path, _)| path.contains("new_name"));
    assert!(
        old_appears && new_appears,
        "both old_name and new_name should appear when canonical_lineage is off; got: {rows_off:?}"
    );
}

// ---------------------------------------------------------------------------
// Recycled-filename tests: the rename map must be applied per retirement
// epoch, never by name alone. A new, unrelated file that reuses a retired
// name keeps its own history; only rows that belong to the retired file's
// lifetime fold onto the lineage target.
// ---------------------------------------------------------------------------

fn git(dir: &std::path::Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .status()
        .expect("spawn git");
    assert!(status.success(), "git {args:?} failed");
}

fn write(root: &std::path::Path, rel: &str, content: &str) {
    std::fs::write(root.join(rel), content).unwrap();
}

fn commit_at(dir: &std::path::Path, day: u32, msg: &str) {
    git(dir, &["add", "-A"]);
    let stamp = format!("2026-03-{day:02}T12:00:00Z");
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["commit", "-m", msg, "--quiet"])
        .env("GIT_AUTHOR_DATE", &stamp)
        .env("GIT_COMMITTER_DATE", &stamp)
        .status()
        .expect("spawn git commit");
    assert!(status.success(), "git commit {msg} failed");
}

#[test]
fn recycled_filename_keeps_its_own_history() {
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    git(p, &["init", "-b", "main", "--quiet"]);
    git(p, &["config", "user.email", "t@example.com"]);
    git(p, &["config", "user.name", "T"]);

    // Epoch 1: `a.rs` lives, gets a revision, and is renamed to `b.rs`.
    write(
        p,
        "a.rs",
        "fn original() { println!(\"the first file\"); }\n",
    );
    commit_at(p, 1, "add a.rs");
    write(
        p,
        "a.rs",
        "fn original() { println!(\"the first file\"); }\nfn more() {}\n",
    );
    commit_at(p, 2, "modify a.rs");
    git(p, &["mv", "a.rs", "b.rs"]);
    commit_at(p, 3, "rename a.rs to b.rs");
    // Epoch 2: an unrelated file recycles the freed name.
    write(
        p,
        "a.rs",
        "struct Unrelated;\nimpl Unrelated { fn nothing() {} }\n",
    );
    commit_at(p, 4, "add a NEW unrelated a.rs");
    write(
        p,
        "a.rs",
        "struct Unrelated;\nimpl Unrelated { fn nothing() {} fn also() {} }\n",
    );
    commit_at(p, 5, "modify the new a.rs");
    write(
        p,
        "b.rs",
        "fn original() { println!(\"the first file\"); }\nfn more() {}\nfn last() {}\n",
    );
    commit_at(p, 6, "modify b.rs");

    let repo = GixRepo::open(p).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: p.to_path_buf(),
        min_revs: 1,
        use_canonical_lineage: true,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");
    let rows = run_revisions(&db, &opts).expect("revisions");

    // The recycled `a.rs` is a different file: its two revisions must NOT
    // fold onto `b.rs`, and it must not vanish from the output.
    let a_count = rows.iter().find(|(p, _)| p == "a.rs").map(|(_, n)| *n);
    assert_eq!(
        a_count,
        Some(2),
        "the NEW a.rs must keep its own two revisions; got rows: {rows:?}"
    );
    // The retired file's history (add + modify + rename row + later change)
    // aggregates under the canonical name.
    let b_count = rows.iter().find(|(p, _)| p == "b.rs").map(|(_, n)| *n);
    assert_eq!(
        b_count,
        Some(4),
        "b.rs must aggregate exactly the retired file's four revisions; got rows: {rows:?}"
    );
}

/// Seed the fact tables directly (newest commit inserted FIRST, matching the
/// reverse-chronological ingest walk so smaller rowid = newer commit) and
/// check the epoch map end to end: a name retired twice maps each epoch's
/// rows to that epoch's own canonical target.
#[test]
fn a_name_retired_twice_maps_each_epoch_to_its_own_canonical() {
    let db = FactsDb::new_in_memory().expect("db");
    let commit = |rev: &str, day: u32| {
        format!(
            "INSERT INTO commits (rev, author_email, author_name, committer_email, \
             canonical_author, date, committer_date, message, is_merge, parent_count) \
             VALUES ('{rev}', 'a@x', 'A', 'a@x', 'a@x', \
             '2026-03-{day:02} 12:00:00', '2026-03-{day:02} 12:00:00', 'm', false, 1)"
        )
    };
    // Newest first: c4 (rename a->c), c3 (recreate a), c2 (rename a->b), c1 (add a).
    for stmt in [
        commit("c4", 4),
        commit("c3", 3),
        commit("c2", 2),
        commit("c1", 1),
        "INSERT INTO changes VALUES ('c1', 'a.rs', 'added', NULL, 1, 0)".to_string(),
        "INSERT INTO changes VALUES ('c2', 'b.rs', 'renamed', 'a.rs', 0, 0)".to_string(),
        "INSERT INTO changes VALUES ('c3', 'a.rs', 'added', NULL, 1, 0)".to_string(),
        "INSERT INTO changes VALUES ('c4', 'c.rs', 'renamed', 'a.rs', 0, 0)".to_string(),
    ] {
        db.execute_batch(&stmt).expect("seed");
    }
    codelore_lib::facts::ingest::materialize_changes_lineage(&db).expect("materialize");

    let path_of = |rev: &str| {
        db.query_row(
            "SELECT path FROM changes_lineage WHERE rev = ?",
            [rev],
            |r| r.get::<_, String>(0),
        )
        .expect("query")
    };
    assert_eq!(
        path_of("c1"),
        "b.rs",
        "epoch-1 row folds onto epoch 1's target"
    );
    assert_eq!(
        path_of("c3"),
        "c.rs",
        "epoch-2 row folds onto epoch 2's target"
    );
    let epochs: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM path_lineage WHERE old_path = 'a.rs'",
            [],
            |r| r.get(0),
        )
        .expect("count");
    assert_eq!(
        epochs, 2,
        "a twice-retired name carries one map row per epoch"
    );
}

/// A `copied` row also carries `rename_from`, but a copy does not retire its
/// source: it must neither seed a lineage chain nor rewrite the source's rows.
#[test]
fn copied_rows_do_not_seed_lineage() {
    let db = FactsDb::new_in_memory().expect("db");
    for stmt in [
        "INSERT INTO commits (rev, author_email, author_name, committer_email, \
         canonical_author, date, committer_date, message, is_merge, parent_count) \
         VALUES ('c2', 'a@x', 'A', 'a@x', 'a@x', \
         '2026-03-02 12:00:00', '2026-03-02 12:00:00', 'm', false, 1)",
        "INSERT INTO commits (rev, author_email, author_name, committer_email, \
         canonical_author, date, committer_date, message, is_merge, parent_count) \
         VALUES ('c1', 'a@x', 'A', 'a@x', 'a@x', \
         '2026-03-01 12:00:00', '2026-03-01 12:00:00', 'm', false, 1)",
        "INSERT INTO changes VALUES ('c1', 'src.rs', 'added', NULL, 1, 0)",
        "INSERT INTO changes VALUES ('c2', 'dup.rs', 'copied', 'src.rs', 1, 0)",
    ] {
        db.execute_batch(stmt).expect("seed");
    }
    codelore_lib::facts::ingest::materialize_changes_lineage(&db).expect("materialize");

    let edges: i64 = db
        .query_row("SELECT COUNT(*) FROM path_lineage", [], |r| r.get(0))
        .expect("count");
    assert_eq!(edges, 0, "a copy must not create a lineage edge");
    let src_path: String = db
        .query_row(
            "SELECT path FROM changes_lineage WHERE rev = 'c1'",
            [],
            |r| r.get(0),
        )
        .expect("query");
    assert_eq!(src_path, "src.rs", "the copy source keeps its own identity");
}
