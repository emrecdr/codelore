//! `authors` analysis tests (Plan 8 §2 Task 6).

use codelore_lib::Options;
use codelore_lib::analyses::authors::run_authors;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;

#[test]
fn authors_groups_by_canonical_author_and_sorts_desc() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_authors(&db, &opts).expect("run authors");
    // tiny_repo: 5 commits, 1 author ("Tiny" / tiny@example.com).
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].commits, 5);
}

#[test]
fn authors_against_differential_fixture() {
    let fixture = codelore_lib::test_support::differential_repo::build();
    let repo = GixRepo::open(fixture.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: fixture.dir.path().to_path_buf(),
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_authors(&db, &opts).expect("run authors");
    // differential_repo has 5 raw author identities across its commits:
    //   - alice-old@example.com (→ canonical-alice@example.com via .mailmap)
    //   - bob-aliased@example.com (→ canonical-bob@example.com via .mailmap)
    //   - c.lee@example.com (→ carol@example.com via .mailmap)
    //   - 49699333+dependabot[bot]@users.noreply.github.com (bot, no mailmap)
    //   - noop@example.com — the implicit committer of the `git merge --no-ff`
    //     merge commit, which inherits the repo's `user.email` config because
    //     `git merge` doesn't accept a `--author` flag.
    //
    // With default `Options { include_merges: false, .. }` the merge commit is
    // filtered out at repo-walk time (matches the spec §3.1 default and the
    // GitCliRepo backend's `--no-merges` semantics), so the merge-committer
    // never gets ingested and the analysis sees 4 distinct authors. The
    // 5-author behavior is exercised by the `_with_merges_included` test
    // below to give R4 (merge-filter) positive coverage in both modes.
    assert_eq!(
        rows.len(),
        4,
        "expected 4 distinct canonical authors with merges filtered (3 humans + 1 bot), got {}: {:?}",
        rows.len(),
        rows.iter().map(|r| &r.author).collect::<Vec<_>>()
    );
    let names: std::collections::HashSet<&str> = rows.iter().map(|r| r.author.as_str()).collect();
    assert!(
        names.contains("canonical-alice@example.com"),
        "Alice's old email should canonicalize via .mailmap"
    );
    assert!(
        names.contains("canonical-bob@example.com"),
        "Bob's aliased email should canonicalize via .mailmap"
    );
    assert!(
        names.contains("carol@example.com"),
        "Carol's email should canonicalize via .mailmap"
    );
    // Sorted desc by commit count
    for w in rows.windows(2) {
        assert!(
            w[0].commits >= w[1].commits,
            "authors must be sorted by commits desc; got {} then {}",
            w[0].commits,
            w[1].commits
        );
    }
}

/// R4 positive-coverage test: with `include_merges = true`, the merge
/// commit is ingested and its committer appears as a 5th distinct author
/// (the implicit `noop@example.com` set by `git config user.email` during
/// fixture setup; `git merge --no-ff` doesn't accept an `--author` flag
/// so the merge inherits the repo config). The default-filtered behavior
/// (4 authors) is asserted in `authors_against_differential_fixture`
/// above; this one asserts the flag actually flips the behavior.
#[test]
fn authors_against_differential_fixture_with_merges_included() {
    let fixture = codelore_lib::test_support::differential_repo::build();
    let repo = GixRepo::open(fixture.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: fixture.dir.path().to_path_buf(),
        include_merges: true,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_authors(&db, &opts).expect("run authors");
    assert_eq!(
        rows.len(),
        5,
        "with include_merges=true expected 5 distinct authors (3 humans + 1 bot + 1 merge-committer), got {}: {:?}",
        rows.len(),
        rows.iter().map(|r| &r.author).collect::<Vec<_>>()
    );
    let names: std::collections::HashSet<&str> = rows.iter().map(|r| r.author.as_str()).collect();
    assert!(
        names.contains("noop@example.com"),
        "merge-committer (noop@example.com) should appear when merges are included"
    );
}

#[test]
fn authors_rows_limit_caps_output() {
    let fixture = codelore_lib::test_support::differential_repo::build();
    let repo = GixRepo::open(fixture.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: fixture.dir.path().to_path_buf(),
        rows_limit: Some(2),
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_authors(&db, &opts).expect("run authors");
    assert!(rows.len() <= 2, "rows_limit should cap output");
}
