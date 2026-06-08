//! `soc` (Sum of Coupling) analysis integration tests.
//!
//! Validates the formula: each commit of size N contributes (N-1) to every
//! entity in it. Solo commits contribute 0.

use codelore_lib::Options;
use codelore_lib::analyses::soc::run_soc;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;

fn run_git(path: &std::path::Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(path)
        .output()
        .expect("git");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn write(p: std::path::PathBuf, content: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

/// Fixture:
///   - commit 1 touches `{a, b, c}`     → each gets +2 `SoC`
///   - commit 2 touches `{a, b}`        → each gets +1 `SoC`
///   - commit 3 touches `{c}`           → +0 (solo commit)
///   - commit 4 touches `{a, b, c, d}`  → each gets +3 `SoC`
///
/// Expected `SoC`: a=6, b=6, c=5, d=3
#[test]
fn soc_formula_each_commit_contributes_size_minus_one() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    run_git(path, &["init", "-b", "main", "--quiet"]);
    run_git(path, &["config", "user.email", "t@e.com"]);
    run_git(path, &["config", "user.name", "T"]);

    // c1 — {a, b, c}
    write(path.join("a.txt"), "v1\n");
    write(path.join("b.txt"), "v1\n");
    write(path.join("c.txt"), "v1\n");
    run_git(path, &["add", "."]);
    run_git(path, &["commit", "-m", "c1: a+b+c", "--quiet"]);

    // c2 — {a, b}
    write(path.join("a.txt"), "v2\n");
    write(path.join("b.txt"), "v2\n");
    run_git(path, &["add", "."]);
    run_git(path, &["commit", "-m", "c2: a+b", "--quiet"]);

    // c3 — {c} solo
    write(path.join("c.txt"), "v2\n");
    run_git(path, &["add", "."]);
    run_git(path, &["commit", "-m", "c3: c", "--quiet"]);

    // c4 — {a, b, c, d}
    write(path.join("a.txt"), "v3\n");
    write(path.join("b.txt"), "v3\n");
    write(path.join("c.txt"), "v3\n");
    write(path.join("d.txt"), "v1\n");
    run_git(path, &["add", "."]);
    run_git(path, &["commit", "-m", "c4: a+b+c+d", "--quiet"]);

    let repo = GixRepo::open(path).expect("gix open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: path.to_path_buf(),
        min_revs: 0, // include all paths regardless of revision count
        min_soc: Some(0), // include solo-commit paths in output
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_soc(&db, &opts).expect("soc");

    let soc = |e: &str| -> u32 {
        rows.iter()
            .find(|r| r.entity == e)
            .map_or(0, |r| r.soc)
    };

    assert_eq!(soc("a.txt"), 6, "a.txt: 2 + 1 + 0 + 3 = 6 (got {})", soc("a.txt"));
    assert_eq!(soc("b.txt"), 6, "b.txt: 2 + 1 + 0 + 3 = 6 (got {})", soc("b.txt"));
    assert_eq!(soc("c.txt"), 5, "c.txt: 2 + 0 + 0 + 3 = 5 (got {})", soc("c.txt"));
    assert_eq!(soc("d.txt"), 3, "d.txt: only c4 → +3 (got {})", soc("d.txt"));

    // Output is sorted by soc DESC, entity ASC.
    // Expected order: a (6), b (6), c (5), d (3).
    let entities: Vec<&str> = rows.iter().map(|r| r.entity.as_str()).collect();
    assert_eq!(entities, ["a.txt", "b.txt", "c.txt", "d.txt"]);
}

/// `--min-soc N` should drop entities below the threshold.
#[test]
fn soc_min_soc_filter_drops_low_scoring_entities() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    run_git(path, &["init", "-b", "main", "--quiet"]);
    run_git(path, &["config", "user.email", "t@e.com"]);
    run_git(path, &["config", "user.name", "T"]);

    // Two paired commits over {a, b}; one solo over {c}.
    for v in 1..=2 {
        write(path.join("a.txt"), &format!("{v}\n"));
        write(path.join("b.txt"), &format!("{v}\n"));
        run_git(path, &["add", "."]);
        run_git(path, &["commit", "-m", &format!("paired {v}"), "--quiet"]);
    }
    write(path.join("c.txt"), "solo\n");
    run_git(path, &["add", "."]);
    run_git(path, &["commit", "-m", "solo", "--quiet"]);

    let repo = GixRepo::open(path).expect("gix open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: path.to_path_buf(),
        min_revs: 0,
        min_soc: Some(2), // drop anything below soc=2
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_soc(&db, &opts).expect("soc");
    // a + b each have SoC = 2; c has SoC = 0 (solo); --min-soc 2 keeps a, b.
    let entities: Vec<&str> = rows.iter().map(|r| r.entity.as_str()).collect();
    assert!(entities.contains(&"a.txt"));
    assert!(entities.contains(&"b.txt"));
    assert!(!entities.contains(&"c.txt"), "solo path should be dropped by --min-soc 2");
}
