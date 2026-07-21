use codelore_lib::Options;
use codelore_lib::analyses::communication::run_communication;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;

#[test]
fn communication_for_tiny_repo_with_single_author() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_shared_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_communication(&db, &opts).expect("run");
    // tiny_repo has 1 author → 0 pairs (self-pair excluded). Empty result is correct.
    assert!(
        rows.is_empty(),
        "single-author repo should produce no communication pairs, got {} rows",
        rows.len()
    );
}

#[test]
fn communication_row_shape() {
    use codelore_lib::analyses::communication::CommunicationRow;
    let row = CommunicationRow {
        author_a: "a@b.com".into(),
        author_b: "c@d.com".into(),
        shared: 3,
        average: 5,
        strength: 60.0,
    };
    assert_eq!(row.author_a, "a@b.com");
    assert_eq!(row.shared, 3);
}

/// Under `--code-maat-compat`, strength truncates (`(int …)`) AND divides
/// shared by the CEIL'd average (code-maat's `average-commits`), not the raw
/// mean. Fixture: Alice makes 2 commits touching {f1,f2}; Bob makes 3 commits
/// touching {f1,f2,f3}. Then shared = 2, average = ceil((2+3)/2) = 3, and the
/// expected strength = floor(100*2/3) = 66. (Pre-fix ROUND-of-raw-mean → 80;
/// FLOOR-of-raw-mean → 80; full fix → 66.)
#[test]
fn communication_strength_truncates_over_ceiled_average_under_compat() {
    use std::process::Command;

    fn run_git(path: &std::path::Path, name: &str, email: &str, date: &str, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(path)
            .args(args)
            .env("GIT_AUTHOR_NAME", name)
            .env("GIT_AUTHOR_EMAIL", email)
            .env("GIT_COMMITTER_NAME", name)
            .env("GIT_COMMITTER_EMAIL", email)
            .env("GIT_AUTHOR_DATE", date)
            .env("GIT_COMMITTER_DATE", date)
            .status()
            .expect("git");
        assert!(status.success());
    }

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path();
    run_git(
        path,
        "Alice",
        "alice@t",
        "2026-02-01T00:00:00Z",
        &["init", "-b", "main", "--quiet"],
    );

    let commit = |name: &str, email: &str, day: u32, files: &[(&str, &str)]| {
        let date = format!("2026-02-{day:02}T12:00:00Z");
        for (fname, content) in files {
            std::fs::write(path.join(fname), content).unwrap();
        }
        run_git(path, name, email, &date, &["add", "."]);
        run_git(
            path,
            name,
            email,
            &date,
            &["commit", "-m", &format!("c{day}"), "--quiet"],
        );
    };

    // Alice: 2 commits touching {f1, f2}. Bob: 3 commits touching {f1, f2, f3}.
    commit("Alice", "alice@t", 1, &[("f1.txt", "f1-alice")]);
    commit("Alice", "alice@t", 2, &[("f2.txt", "f2-alice")]);
    commit("Bob", "bob@t", 3, &[("f1.txt", "f1-bob")]);
    commit("Bob", "bob@t", 4, &[("f2.txt", "f2-bob")]);
    commit("Bob", "bob@t", 5, &[("f3.txt", "f3-bob")]);

    let repo = GixRepo::open(path).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: path.to_path_buf(),
        min_shared_revs: 1,
        code_maat_compat: true,
        use_canonical_lineage: false,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_communication(&db, &opts).expect("communication");
    let pair = rows
        .iter()
        .find(|r| r.shared == 2)
        .expect("the (Alice, Bob) pair");
    assert_eq!(pair.average, 3, "ceil((2+3)/2)");
    assert!(
        (pair.strength - 66.0).abs() < f64::EPSILON,
        "compat strength = int over the CEIL'd average; got {}",
        pair.strength
    );
}
