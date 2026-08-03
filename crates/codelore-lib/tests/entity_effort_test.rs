//! Behavioral coverage for `run_entity_effort` (`entity-effort` — per-
//! `(entity, author)` revision counts alongside each entity's total
//! revisions). Previously only touched by dispatch-metadata loops (name
//! resolution, CSV header shape); never run against a real ingested history
//! and asserted for correctness.
//!
//! The fixture mixes two authors across two files, one of which is renamed
//! partway through — so the same test also exercises the `changes_lineage`
//! rewrite `entity-effort` opts into via `analyses::lineage::rewrite`.

use std::collections::HashMap;
use std::process::Command;

use codelore_lib::Options;
use codelore_lib::analyses::entity_effort::run_entity_effort;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;

/// Run one git command in `path` with an explicit author/committer identity
/// and fixed dates, so a single fixture can mix multiple authors
/// deterministically. Mirrors `bus_factor_test.rs::git_as`.
fn git_as(path: &std::path::Path, date: &str, author: &str, email: &str, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(args)
        .env("GIT_AUTHOR_NAME", author)
        .env("GIT_AUTHOR_EMAIL", email)
        .env("GIT_COMMITTER_NAME", author)
        .env("GIT_COMMITTER_EMAIL", email)
        .env("GIT_AUTHOR_DATE", date)
        .env("GIT_COMMITTER_DATE", date)
        .status()
        .expect("git");
    assert!(status.success(), "git {args:?} failed");
}

/// Build a 5-commit, 2-author fixture:
///
/// 1. Alice creates `old.rs` → (old.rs, alice) 1 rev
/// 2. Alice creates `other.rs` → (other.rs, alice) 1 rev
/// 3. Bob edits `other.rs` → (other.rs, bob) 1 rev
/// 4. Alice renames `old.rs` → `new.rs` (`git mv`) →
///    (new.rs, alice) 1 rev, `rename_from` = `old.rs`
/// 5. Bob edits `new.rs` → (new.rs, bob) 1 rev
///
/// Without canonical lineage, `old.rs` and `new.rs` are distinct entities
/// (`old.rs` `total_revs` = 1, `new.rs` `total_revs` = 2). With it, `old.rs`'s
/// single revision folds into `new.rs` (alice's revs there become 2,
/// `new.rs` `total_revs` = 3).
fn build_two_author_rename_repo() -> tempfile::TempDir {
    const ALICE: (&str, &str) = ("Alice", "alice@example.com");
    const BOB: (&str, &str) = ("Bob", "bob@example.com");

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path();

    git_as(
        path,
        "2026-01-01T00:00:00Z",
        ALICE.0,
        ALICE.1,
        &["init", "-b", "main", "--quiet"],
    );

    // Commit 1: Alice creates old.rs.
    std::fs::write(path.join("old.rs"), "one\ntwo\nthree\n").unwrap();
    git_as(
        path,
        "2026-01-01T00:00:00Z",
        ALICE.0,
        ALICE.1,
        &["add", "old.rs"],
    );
    git_as(
        path,
        "2026-01-01T00:00:00Z",
        ALICE.0,
        ALICE.1,
        &["commit", "-m", "create old.rs", "--quiet"],
    );

    // Commit 2: Alice creates other.rs.
    std::fs::write(path.join("other.rs"), "alpha\nbeta\n").unwrap();
    git_as(
        path,
        "2026-01-02T00:00:00Z",
        ALICE.0,
        ALICE.1,
        &["add", "other.rs"],
    );
    git_as(
        path,
        "2026-01-02T00:00:00Z",
        ALICE.0,
        ALICE.1,
        &["commit", "-m", "create other.rs", "--quiet"],
    );

    // Commit 3: Bob edits other.rs.
    std::fs::write(path.join("other.rs"), "alpha\ngamma\n").unwrap();
    git_as(
        path,
        "2026-01-03T00:00:00Z",
        BOB.0,
        BOB.1,
        &["commit", "-am", "edit other.rs", "--quiet"],
    );

    // Commit 4: Alice renames old.rs -> new.rs, no content change.
    git_as(
        path,
        "2026-01-04T00:00:00Z",
        ALICE.0,
        ALICE.1,
        &["mv", "old.rs", "new.rs"],
    );
    git_as(
        path,
        "2026-01-04T00:00:00Z",
        ALICE.0,
        ALICE.1,
        &["commit", "-m", "rename old.rs to new.rs", "--quiet"],
    );

    // Commit 5: Bob edits new.rs.
    std::fs::write(path.join("new.rs"), "one\ntwo\nthree\nfour\nfive\n").unwrap();
    git_as(
        path,
        "2026-01-05T00:00:00Z",
        BOB.0,
        BOB.1,
        &["commit", "-am", "edit new.rs", "--quiet"],
    );

    dir
}

/// Collect rows into `(entity, author) -> (author_revs, total_revs)`,
/// panicking on a duplicate key (the SQL groups by exactly this pair, so a
/// duplicate would indicate the aggregation broke).
fn rows_by_entity_author(
    rows: &[codelore_lib::analyses::entity_effort::EntityEffortRow],
) -> HashMap<(String, String), (u32, u32)> {
    let mut map = HashMap::new();
    for row in rows {
        let prev = map.insert(
            (row.entity.clone(), row.author.clone()),
            (row.author_revs, row.total_revs),
        );
        assert!(
            prev.is_none(),
            "duplicate (entity, author) row for ({}, {}); rows: {rows:?}",
            row.entity,
            row.author
        );
    }
    map
}

#[test]
fn entity_effort_without_lineage_splits_pre_and_post_rename_rows() {
    let fixture = build_two_author_rename_repo();
    let repo = GixRepo::open(fixture.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: fixture.path().to_path_buf(),
        min_revs: 1,
        use_canonical_lineage: false,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_entity_effort(&db, &opts).expect("run entity-effort");
    let by_key = rows_by_entity_author(&rows);

    assert_eq!(
        by_key.len(),
        5,
        "expected 5 distinct (entity, author) rows without lineage; got {rows:?}"
    );
    assert_eq!(
        by_key.get(&("old.rs".to_string(), "alice@example.com".to_string())),
        Some(&(1, 1)),
        "old.rs/alice: author_revs=1, total_revs=1 (sole author): {rows:?}"
    );
    assert_eq!(
        by_key.get(&("other.rs".to_string(), "alice@example.com".to_string())),
        Some(&(1, 2)),
        "other.rs/alice: {rows:?}"
    );
    assert_eq!(
        by_key.get(&("other.rs".to_string(), "bob@example.com".to_string())),
        Some(&(1, 2)),
        "other.rs/bob: {rows:?}"
    );
    assert_eq!(
        by_key.get(&("new.rs".to_string(), "alice@example.com".to_string())),
        Some(&(1, 2)),
        "new.rs/alice (rename-only commit): {rows:?}"
    );
    assert_eq!(
        by_key.get(&("new.rs".to_string(), "bob@example.com".to_string())),
        Some(&(1, 2)),
        "new.rs/bob: {rows:?}"
    );
}

#[test]
fn entity_effort_with_lineage_merges_renamed_entity_revisions() {
    let fixture = build_two_author_rename_repo();
    let repo = GixRepo::open(fixture.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: fixture.path().to_path_buf(),
        min_revs: 1,
        use_canonical_lineage: true,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_entity_effort(&db, &opts).expect("run entity-effort under lineage");
    let by_key = rows_by_entity_author(&rows);

    // old.rs must not appear at all — its sole revision folds into new.rs.
    assert!(
        !rows.iter().any(|r| r.entity == "old.rs"),
        "old.rs must not appear under canonical lineage; got {rows:?}"
    );
    assert_eq!(
        by_key.len(),
        4,
        "expected 4 distinct (entity, author) rows under lineage (old.rs/alice \
         merges into new.rs/alice, dropping the old.rs row but not the pair \
         count for the other 3 untouched pairs); got {rows:?}"
    );
    // alice's pre-rename old.rs revision + the rename commit itself both
    // canonicalize to new.rs, so author_revs = 2; total_revs (alice's 2 +
    // bob's 1) = 3.
    assert_eq!(
        by_key.get(&("new.rs".to_string(), "alice@example.com".to_string())),
        Some(&(2, 3)),
        "new.rs/alice must aggregate the pre-rename old.rs revision: {rows:?}"
    );
    assert_eq!(
        by_key.get(&("new.rs".to_string(), "bob@example.com".to_string())),
        Some(&(1, 3)),
        "new.rs/bob unaffected by the rename: {rows:?}"
    );
    assert_eq!(
        by_key.get(&("other.rs".to_string(), "alice@example.com".to_string())),
        Some(&(1, 2)),
        "other.rs/alice unaffected by the rename: {rows:?}"
    );
    assert_eq!(
        by_key.get(&("other.rs".to_string(), "bob@example.com".to_string())),
        Some(&(1, 2)),
        "other.rs/bob unaffected by the rename: {rows:?}"
    );
}
