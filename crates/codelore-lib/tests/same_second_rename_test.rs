//! F22 regression — same-second rename chains must traverse the full
//! lineage when `--canonical-lineage` is on.
//!
//! Earlier the recursive CTE used strict `co.date > l.current_date` to
//! extend the chain, which terminated prematurely if two sequential
//! renames (`A → B` then `B → C`) landed in commits sharing the exact
//! same second. The fix threads `commits.rowid` through the CTE and
//! breaks date-ties via `co.rowid < l.current_rowid` — gix walks
//! reverse-chronologically so newer commits receive smaller rowids.

use codelore_lib::Options;
use codelore_lib::analyses::revisions::run_revisions;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;
use std::process::Command;
use tempfile::TempDir;

/// Build a tiny repo with a two-step rename chain `a.txt → b.txt → c.txt`
/// where both rename commits share the same second. Returns the tempdir.
fn build_same_second_rename_chain() -> TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path();

    let git = |args: &[&str]| -> bool {
        Command::new("git")
            .arg("-C")
            .arg(path)
            .args(args)
            .status()
            .expect("spawn git")
            .success()
    };
    let git_commit = |msg: &str, date: &str| {
        let ok = Command::new("git")
            .arg("-C")
            .arg(path)
            .args(["commit", "-m", msg, "--quiet"])
            .env("GIT_AUTHOR_DATE", date)
            .env("GIT_COMMITTER_DATE", date)
            .status()
            .expect("spawn git commit")
            .success();
        assert!(ok, "git commit failed: {msg}");
    };

    assert!(git(&["init", "-b", "main", "--quiet"]));
    assert!(git(&["config", "user.email", "rename@example.com"]));
    assert!(git(&["config", "user.name", "Renamer"]));

    // Seed: a.txt with enough content to register as a rename rather than
    // a copy on `git diff -M`.
    std::fs::write(path.join("a.txt"), "alpha\nbeta\ngamma\ndelta\n").unwrap();
    assert!(git(&["add", "a.txt"]));
    git_commit("seed", "2026-04-01T12:00:00Z");

    // Same-second window for the two renames. The commits are committed
    // sequentially but share identical second-level timestamps, exercising
    // the date-tie branch of the recursive CTE.
    let same_second = "2026-04-01T13:00:00Z";

    // Step 1: a.txt → b.txt
    assert!(git(&["mv", "a.txt", "b.txt"]));
    git_commit("rename a -> b", same_second);

    // Step 2: b.txt → c.txt at the SAME second
    assert!(git(&["mv", "b.txt", "c.txt"]));
    git_commit("rename b -> c", same_second);

    dir
}

#[test]
fn same_second_rename_chain_merges_under_canonical_path() {
    let fixture = build_same_second_rename_chain();
    let repo = GixRepo::open(fixture.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");

    let opts = Options {
        repo_path: fixture.path().to_path_buf(),
        min_revs: 1,
        use_canonical_lineage: true,
        ..Default::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_revisions(&db, &opts).expect("revisions");

    // With the F22 fix, the lineage must traverse a.txt → b.txt → c.txt
    // even though b → c shares the same second as a → b. Result: only
    // c.txt appears (the canonical path), with 3 revisions accumulated.
    let by_path: std::collections::HashMap<_, _> = rows
        .iter()
        .map(|(path, n_revs)| (path.clone(), *n_revs))
        .collect();
    assert!(
        !by_path.contains_key("a.txt"),
        "a.txt should be merged into c.txt under canonical_lineage; got rows: {rows:?}"
    );
    assert!(
        !by_path.contains_key("b.txt"),
        "b.txt should be merged into c.txt (same-second chain must NOT \
         terminate at b.txt); got rows: {rows:?}"
    );
    let c_revs = by_path
        .get("c.txt")
        .copied()
        .unwrap_or_else(|| panic!("c.txt missing; got rows: {rows:?}"));
    assert_eq!(
        c_revs, 3,
        "c.txt should accumulate revs from a.txt + b.txt + c.txt (3 total); \
         got {c_revs} from rows: {rows:?}"
    );
}
