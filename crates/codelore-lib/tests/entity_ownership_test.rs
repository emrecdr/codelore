//! Behavioral coverage for `run_entity_ownership` (`entity-ownership` — the
//! per-`(entity, author)` added/deleted churn breakdown). Previously only
//! touched by dispatch-metadata loops (name resolution, CSV header shape);
//! never run against a real ingested history and asserted for correctness.
//!
//! The fixture mixes two authors across two files, one of which is renamed
//! partway through — so the same test also exercises the `changes_lineage`
//! rewrite `entity-ownership` opts into via `analyses::lineage::rewrite`.

use std::collections::HashMap;
use std::process::Command;

use codelore_lib::Options;
use codelore_lib::analyses::entity_ownership::run_entity_ownership;
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
/// 1. Alice creates `old.rs` (3 lines) → (old.rs, alice) +3/-0
/// 2. Alice creates `other.rs` (2 lines) → (other.rs, alice) +2/-0
/// 3. Bob edits `other.rs` (1 line replaced) → (other.rs, bob) +1/-1
/// 4. Alice renames `old.rs` → `new.rs` (no content change, `git mv`) →
///    (new.rs, alice) +0/-0, `rename_from` = `old.rs`
/// 5. Bob appends 2 lines to `new.rs` → (new.rs, bob) +2/-0
///
/// Without canonical lineage, `old.rs` and `new.rs` are distinct entities.
/// With it, `old.rs`'s pre-rename churn folds into `new.rs`.
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

    // Commit 1: Alice creates old.rs (3 lines).
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

    // Commit 2: Alice creates other.rs (2 lines).
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

    // Commit 3: Bob replaces line 2 of other.rs (1 add, 1 delete).
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

    // Commit 5: Bob appends 2 lines to new.rs.
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

/// Collect rows into `(entity, author) -> (added, deleted)`, panicking on a
/// duplicate key (the SQL groups by exactly this pair, so a duplicate would
/// indicate the aggregation broke).
fn rows_by_entity_author(
    rows: &[codelore_lib::analyses::entity_ownership::EntityOwnershipRow],
) -> HashMap<(String, String), (u64, u64)> {
    let mut map = HashMap::new();
    for row in rows {
        let prev = map.insert(
            (row.entity.clone(), row.author.clone()),
            (row.added, row.deleted),
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
fn entity_ownership_without_lineage_splits_pre_and_post_rename_rows() {
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

    let rows = run_entity_ownership(&db, &opts).expect("run entity-ownership");
    let by_key = rows_by_entity_author(&rows);

    assert_eq!(
        by_key.len(),
        5,
        "expected 5 distinct (entity, author) rows without lineage; got {rows:?}"
    );
    assert_eq!(
        by_key.get(&("old.rs".to_string(), "alice@example.com".to_string())),
        Some(&(3, 0)),
        "old.rs/alice: {rows:?}"
    );
    assert_eq!(
        by_key.get(&("other.rs".to_string(), "alice@example.com".to_string())),
        Some(&(2, 0)),
        "other.rs/alice: {rows:?}"
    );
    assert_eq!(
        by_key.get(&("other.rs".to_string(), "bob@example.com".to_string())),
        Some(&(1, 1)),
        "other.rs/bob: {rows:?}"
    );
    assert_eq!(
        by_key.get(&("new.rs".to_string(), "alice@example.com".to_string())),
        Some(&(0, 0)),
        "new.rs/alice (rename-only commit): {rows:?}"
    );
    assert_eq!(
        by_key.get(&("new.rs".to_string(), "bob@example.com".to_string())),
        Some(&(2, 0)),
        "new.rs/bob: {rows:?}"
    );
}

#[test]
fn entity_ownership_with_lineage_merges_renamed_entity_churn() {
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

    let rows = run_entity_ownership(&db, &opts).expect("run entity-ownership under lineage");
    let by_key = rows_by_entity_author(&rows);

    // old.rs must not appear at all — its sole author (alice)'s churn folds
    // into new.rs.
    assert!(
        !rows.iter().any(|r| r.entity == "old.rs"),
        "old.rs must not appear under canonical lineage; got {rows:?}"
    );
    assert_eq!(
        by_key.len(),
        4,
        "expected 4 distinct (entity, author) rows under lineage (old.rs/alice \
         merges into new.rs/alice); got {rows:?}"
    );
    // alice's pre-rename old.rs churn (3/0) + the rename commit itself
    // (0/0) sum onto new.rs.
    assert_eq!(
        by_key.get(&("new.rs".to_string(), "alice@example.com".to_string())),
        Some(&(3, 0)),
        "new.rs/alice must aggregate the pre-rename old.rs churn: {rows:?}"
    );
    assert_eq!(
        by_key.get(&("new.rs".to_string(), "bob@example.com".to_string())),
        Some(&(2, 0)),
        "new.rs/bob unaffected by the rename: {rows:?}"
    );
    assert_eq!(
        by_key.get(&("other.rs".to_string(), "alice@example.com".to_string())),
        Some(&(2, 0)),
        "other.rs/alice unaffected by the rename: {rows:?}"
    );
    assert_eq!(
        by_key.get(&("other.rs".to_string(), "bob@example.com".to_string())),
        Some(&(1, 1)),
        "other.rs/bob unaffected by the rename: {rows:?}"
    );
}
