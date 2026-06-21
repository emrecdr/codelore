//! End-to-end coverage of `run_pair_programming` against an ingested
//! `FactsDb`. `tiny_repo` has no `Co-Authored-By:` trailers, so the
//! analysis legitimately returns empty — but the SQL must still
//! execute cleanly + the row shape contract must hold.

use codelore_lib::Options;
use codelore_lib::analyses::pair_programming::run_pair_programming;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;

#[test]
fn pair_programming_executes_cleanly_on_tiny_repo() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    // No Co-Authored-By: trailers in `tiny_repo` → no pairs.
    let rows = run_pair_programming(&db, &opts).expect("run pair-programming");
    assert!(
        rows.is_empty(),
        "tiny_repo has no Co-Authored-By: trailers — expected empty pair-programming result, got {} rows",
        rows.len(),
    );
}

#[test]
fn pair_programming_surfaces_real_pairs_from_co_authored_trailers() {
    // Build a tiny repo where one commit carries a Co-Authored-By:
    // trailer. The pair-programming analysis must surface the
    // (primary author, trailer author) pair with pair_commits=1.
    use std::path::Path;
    use std::process::Command;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();

    let git = |args: &[&str]| {
        let s = Command::new("git")
            .arg("-C")
            .arg(path)
            .args(args)
            .status()
            .expect("git");
        assert!(s.success(), "git {args:?} failed");
    };
    let write = |rel: &str, contents: &str| {
        let full = path.join(rel);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(full, contents).unwrap();
    };

    git(&["init", "-b", "main", "--quiet"]);
    git(&["config", "user.email", "primary@example.com"]);
    git(&["config", "user.name", "Primary Dev"]);

    write("README.md", "hello\n");
    git(&["add", "."]);
    // Co-Authored-By: trailer triggers pair-programming detection.
    git(&[
        "commit",
        "-m",
        "first\n\nCo-Authored-By: Pair Dev <pair@example.com>",
        "--quiet",
    ]);

    let repo = GixRepo::open(path as &Path).expect("gix open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: path.to_path_buf(),
        // Default `min_shared_revs = 5` would prune our single-commit
        // pair before it ever leaves the analysis. Drop the floor to 1
        // so the fixture pair surfaces.
        min_shared_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_pair_programming(&db, &opts).expect("run pair-programming");

    // Exactly one pair: (primary@example.com, pair@example.com), 1
    // commit shared.
    assert_eq!(rows.len(), 1, "expected exactly one pair row, got {rows:?}");
    assert_eq!(rows[0].pair_commits, 1, "expected pair_commits=1");
    let pair: Vec<&str> = vec![rows[0].author_a.as_str(), rows[0].author_b.as_str()];
    assert!(
        pair.contains(&"primary@example.com"),
        "primary author missing from pair: {pair:?}",
    );
    assert!(
        pair.contains(&"pair@example.com"),
        "co-authored author missing from pair: {pair:?}",
    );
}

/// Canonical-ordering + dedup regression guard for the integer-interned
/// pair-counter shape. Constructs a fixture where the same author pair
/// (alice ↔ bob) is encountered in DIFFERENT orderings across commits:
/// one commit has alice as primary + bob as co-author; another has bob
/// as primary + alice as co-author. The analysis must emit ONE row for
/// this pair with `pair_commits = 2`, and `author_a < author_b`
/// lexicographically (the analysis's documented canonical contract).
///
/// The prior `HashMap<(String, String), u32>` keyed shape achieved this
/// by sorting the per-commit `participants: Vec<String>` lex-ascending,
/// so the pair tuple always landed as `(alice, bob)` regardless of who
/// was primary. The integer-interned shape sorts indices instead — same
/// invariant per commit, dedup across commits via the append-only
/// interner, but the lex contract is recovered at output time. This
/// test guards both halves.
#[test]
fn pair_programming_dedupes_pair_regardless_of_primary_orientation() {
    use std::process::Command;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();

    let git = |args: &[&str]| {
        let s = Command::new("git")
            .arg("-C")
            .arg(path)
            .args(args)
            .status()
            .expect("git");
        assert!(s.success(), "git {args:?} failed");
    };
    let write = |rel: &str, contents: &str| {
        std::fs::write(path.join(rel), contents).unwrap();
    };

    git(&["init", "-b", "main", "--quiet"]);

    // Commit 1: bob is primary; alice is co-author.
    git(&["config", "user.email", "bob@example.com"]);
    git(&["config", "user.name", "Bob"]);
    write("a.txt", "v1\n");
    git(&["add", "."]);
    git(&[
        "commit",
        "-m",
        "c1\n\nCo-Authored-By: Alice <alice@example.com>",
        "--quiet",
    ]);

    // Commit 2: alice is primary; bob is co-author. Same logical
    // pair, opposite orientation.
    git(&["config", "user.email", "alice@example.com"]);
    git(&["config", "user.name", "Alice"]);
    write("a.txt", "v2\n");
    git(&["add", "."]);
    git(&[
        "commit",
        "-m",
        "c2\n\nCo-Authored-By: Bob <bob@example.com>",
        "--quiet",
    ]);

    let repo = GixRepo::open(path).expect("gix open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: path.to_path_buf(),
        min_shared_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_pair_programming(&db, &opts).expect("run pair-programming");

    assert_eq!(
        rows.len(),
        1,
        "alice↔bob encountered in two orientations must dedup to 1 pair row; got {rows:?}",
    );
    assert_eq!(
        rows[0].pair_commits, 2,
        "both commits should count toward the same pair (got {:?})",
        rows[0],
    );
    // Canonical lex ordering — `author_a < author_b`. alice < bob.
    assert_eq!(rows[0].author_a, "alice@example.com");
    assert_eq!(rows[0].author_b, "bob@example.com");
}
