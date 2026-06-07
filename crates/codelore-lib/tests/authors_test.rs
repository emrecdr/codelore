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
    // differential_repo authors: Alice (alice-old@), Bob (bob-aliased@ →
    // canonical-bob@ via .mailmap), Carol (c.lee@), dependabot[bot], and
    // the implicit `noop@example.com` committer set via `git config user.email`
    // before any --author override. So 5 raw + 0 canonicalized for Alice/Carol.
    //
    // After the Plan 8 §2 T6 mailmap fix (ingest now passes author_name into
    // the gix mailmap lookup), all 3 humans + 1 bot canonicalize correctly:
    //   - alice-old@example.com → canonical-alice@example.com
    //   - bob-aliased@example.com → canonical-bob@example.com
    //   - c.lee@example.com → carol@example.com
    //   - 49699333+dependabot[bot]@users.noreply.github.com (bot, no mailmap)
    //
    // A 5th author appears: `noop@example.com`. This is `git merge --no-ff`'s
    // default author behavior — the merge commit (commit 49) doesn't carry
    // a `--author` override, so it inherits the repo's user.email config
    // (`noop@example.com`, set during `git init` in differential_repo::build).
    // This faithfully mirrors real-world git behavior where merge commits
    // are authored by the developer or CI bot that ran `git merge`. Counting
    // them as a distinct author is correct.
    assert_eq!(
        rows.len(),
        5,
        "expected 5 distinct canonical authors (3 humans + 1 bot + 1 merge-committer), got {}: {:?}",
        rows.len(),
        rows.iter().map(|r| &r.author).collect::<Vec<_>>()
    );
    let names: std::collections::HashSet<&str> =
        rows.iter().map(|r| r.author.as_str()).collect();
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
