//! End-to-end coverage of the `commit_parents` table populated by the ingest
//! pipeline (schema v4).
//!
//! Two fixtures:
//! - `differential_repo`: 50 commits, 1 `--no-ff` merge; verifies the merge
//!   commit's two parents land at positions 0 and 1, and that position 0
//!   matches the first SHA reported by `git log --format=%P`.
//! - `delivery_repo`: ~110 days, 2 `--no-ff` merges; verifies both merge
//!   commits together contribute 4 `commit_parents` rows (2 per merge).
//!
//! Both ingests run with `include_merges: true` — merge commits are filtered
//! at the walker level by default and would produce zero `commit_parents` rows
//! for those commits otherwise.

use codelore_lib::Options;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;

#[test]
fn differential_repo_merge_has_two_parents_in_correct_order() {
    let fixture = codelore_lib::test_support::differential_repo::build();
    let repo = GixRepo::open(fixture.dir.path()).expect("open repo");
    let db = FactsDb::new_in_memory().expect("new_in_memory");
    let opts = Options {
        repo_path: fixture.dir.path().to_path_buf(),
        include_merges: true,
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    // ── Find the single merge commit rev ─────────────────────────────────────
    let merge_rev: String = db
        .query_row(
            "SELECT rev FROM commits WHERE is_merge ORDER BY date LIMIT 1",
            [],
            |r| r.get(0),
        )
        .expect("query merge rev");
    assert!(
        !merge_rev.is_empty(),
        "differential_repo must have at least one merge commit"
    );

    // ── Exactly 2 parent rows for the merge commit ────────────────────────────
    let parent_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM commit_parents WHERE rev = ?",
            [&merge_rev],
            |r| r.get(0),
        )
        .expect("query parent count");
    assert_eq!(
        parent_count, 2,
        "merge commit must have exactly 2 rows in commit_parents"
    );

    // ── Position 0 matches the first parent in git log --format=%P ───────────
    let stored_first_parent: String = db
        .query_row(
            "SELECT parent_rev FROM commit_parents WHERE rev = ? AND position = 0",
            [&merge_rev],
            |r| r.get(0),
        )
        .expect("query position-0 parent");

    let git_parents_out = std::process::Command::new("git")
        .arg("-C")
        .arg(fixture.dir.path())
        .args(["log", "--format=%P", "-1", &merge_rev])
        .output()
        .expect("git log --format=%P");
    let git_parents_line = String::from_utf8(git_parents_out.stdout)
        .expect("utf8")
        .trim()
        .to_string();
    let git_first_parent = git_parents_line
        .split_whitespace()
        .next()
        .expect("merge commit must have at least one parent in git output");

    assert_eq!(
        stored_first_parent, git_first_parent,
        "commit_parents position 0 must match the first parent from `git log --format=%P`"
    );
}

#[test]
fn delivery_repo_two_merges_each_have_two_parent_rows() {
    let fixture = codelore_lib::test_support::delivery_repo::build();
    let repo = GixRepo::open(fixture.dir.path()).expect("open repo");
    let db = FactsDb::new_in_memory().expect("new_in_memory");
    let opts = Options {
        repo_path: fixture.dir.path().to_path_buf(),
        include_merges: true,
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    // 2 merge commits × 2 parents each = 4 rows in commit_parents.
    let total_parent_rows_for_merges: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM commit_parents cp
             JOIN commits c ON c.rev = cp.rev
             WHERE c.is_merge",
            [],
            |r| r.get(0),
        )
        .expect("query total merge parent rows");
    assert_eq!(
        total_parent_rows_for_merges, 4,
        "2 merge commits × 2 parents each = 4 rows in commit_parents"
    );

    // No merge commit may have fewer than 2 parent rows.
    let under_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM (
                 SELECT cp.rev
                 FROM commit_parents cp
                 JOIN commits c ON c.rev = cp.rev
                 WHERE c.is_merge
                 GROUP BY cp.rev
                 HAVING COUNT(*) < 2
             )",
            [],
            |r| r.get(0),
        )
        .expect("query under-count");
    assert_eq!(
        under_count, 0,
        "every merge commit must have at least 2 rows in commit_parents"
    );
}
