//! Smoke tests that verify the `delivery_repo` fixture has the exact shape
//! that later delivery-metrics analyses depend on: 2 merge commits, 3
//! distinct canonical authors, and 4 annotated tags.

use codelore_lib::Options;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;

#[test]
fn delivery_repo_has_two_merges_three_authors_four_tags() {
    let fixture = codelore_lib::test_support::delivery_repo::build();
    let repo = GixRepo::open(fixture.dir.path()).expect("open repo");
    let db = FactsDb::new_in_memory().expect("new_in_memory");
    let opts = Options {
        repo_path: fixture.dir.path().to_path_buf(),
        min_revs: 1,
        // Options::default() has include_merges: false, which drops merge
        // commits at the walker before they reach ingest. Set true so
        // is_merge rows exist.
        include_merges: true,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    // ── 1. Exactly 2 merge commits ─────────────────────────────────────────
    let merge_count: i64 = db
        .query_row("SELECT COUNT(*) FROM commits WHERE is_merge", [], |r| {
            r.get(0)
        })
        .expect("query merge count");
    assert_eq!(
        merge_count, 2,
        "fixture must ingest exactly 2 merge commits"
    );

    // ── 2. Exactly 3 distinct canonical authors (on non-merge commits) ─────
    let author_count: i64 = db
        .query_row(
            "SELECT COUNT(DISTINCT canonical_author) FROM commits WHERE NOT is_merge",
            [],
            |r| r.get(0),
        )
        .expect("query canonical author count");
    assert_eq!(
        author_count, 3,
        "fixture must have exactly 3 distinct canonical authors on non-merge commits"
    );

    // ── 3. Exactly 5 tags: 4 annotated (v0.1.0, v0.2.0, v1.0.0, nightly-1)
    //      + 1 lightweight (light-1, exercising the committer-date fallback)
    let tag_out = std::process::Command::new("git")
        .arg("-C")
        .arg(fixture.dir.path())
        .args(["for-each-ref", "refs/tags", "--format=%(refname)"])
        .output()
        .expect("git for-each-ref");
    let tag_count = String::from_utf8(tag_out.stdout)
        .expect("utf8")
        .lines()
        .filter(|l| !l.is_empty())
        .count();
    assert_eq!(
        tag_count, 5,
        "fixture must have exactly 5 tags (v0.1.0, v0.2.0, v1.0.0, nightly-1, light-1)"
    );
}
